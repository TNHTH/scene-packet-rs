//! 视频分析流水线。

use std::path::Path;

use chrono::Utc;

use crate::asr::AsrService;
use crate::cache::{load_cached_transcript, save_cache};
use crate::config::{effective_cache_dir, load_config, AppConfig};
use crate::error::{no_transcript_asr_not_configured, unsupported_url, AvrError};
use crate::extract::platform::detect_platform;
use crate::extract::ytdlp::YtDlpExtractor;
use crate::ffmpeg::{extract_audio_wav, probe_duration, FfmpegContext};
use crate::jobs::{shared_job_manager, JobManifest, JobProgress};
use crate::media::{media_key, MediaSource};
use crate::models::{
    format_inline_text, AnalyzeResult, CacheMetadata, Transcript, TRANSCRIPT_INLINE_LIMIT,
};
use crate::paths::to_file_uri;
use crate::root::DataRoot;
use crate::VERSION;

pub struct PipelineContext {
    pub root: DataRoot,
    pub config: AppConfig,
}

impl PipelineContext {
    pub fn load(mut root: DataRoot) -> Result<Self, AvrError> {
        root.ensure_layout()?;
        root.path = std::fs::canonicalize(&root.path).map_err(|e| {
            AvrError::new(crate::ErrorCode::IoError, e.to_string(), "检查数据目录。")
        })?;
        let config = load_config(&root)?;
        Ok(Self { root, config })
    }
}

pub async fn analyze_url(ctx: &PipelineContext, url: &str) -> Result<AnalyzeResult, AvrError> {
    if detect_platform(url).is_none() {
        return Err(unsupported_url());
    }

    let base_cache = effective_cache_dir(&ctx.root, &ctx.config);
    let key = processing_key(
        ctx,
        &media_key(
            &MediaSource::Url {
                url: url.to_string(),
            },
            None,
        )?,
    )?;
    let item_cache = base_cache.join(&key);

    let mgr = shared_job_manager();
    let url_clone = url.to_string();
    let ctx_clone = ctx.clone();
    let item_cache_run = item_cache.clone();
    let key_run = key.clone();

    mgr.run_once(&key, &item_cache, || async move {
        run_url_pipeline(&ctx_clone, &url_clone, &item_cache_run, &key_run)
            .await
            .map(|_| ())
    })
    .await?;

    build_result_from_cache(&item_cache, &key)
}

async fn run_url_pipeline(
    ctx: &PipelineContext,
    url: &str,
    item_cache: &Path,
    key: &str,
) -> Result<AnalyzeResult, AvrError> {
    let mgr = shared_job_manager();
    mgr.emit(JobProgress::Stage {
        media_key: key.to_string(),
        stage: "checking_cache".into(),
        percent: 5,
    });

    if let Some(cached) = load_cached_transcript(item_cache)? {
        let (video, trans, _) = cached;
        return Ok(make_analyze_result(&video, &trans, item_cache, key));
    }

    mgr.emit(JobProgress::Stage {
        media_key: key.to_string(),
        stage: "extracting".into(),
        percent: 20,
    });

    let extractor = YtDlpExtractor::new(&ctx.root, &ctx.config)?;
    let (video, info) = extractor.extract_metadata_with_info(url).await?;

    mgr.emit(JobProgress::Stage {
        media_key: key.to_string(),
        stage: "subtitles".into(),
        percent: 40,
    });

    let mut transcript = extractor
        .fetch_subtitles_with_info(url, item_cache, &info)
        .await?;
    let mut asr_provider: Option<String> = Some("subtitle".into());
    let mut asr_model = String::new();

    if transcript.is_none()
        || transcript
            .as_ref()
            .map(|t| t.segments.is_empty())
            .unwrap_or(true)
    {
        let asr = AsrService::new(ctx.root.clone(), ctx.config.clone());
        let preset = asr.active_preset(None, Some(video.platform.as_str()));
        if !asr.is_ready(&preset) {
            return Err(no_transcript_asr_not_configured());
        }
        mgr.emit(JobProgress::Stage {
            media_key: key.to_string(),
            stage: "asr".into(),
            percent: 60,
        });
        let audio_path = item_cache.join("audio.wav");
        extractor.download_audio(url, &audio_path).await?;
        transcript = Some(
            asr.transcribe(&audio_path, None, Some(video.platform.as_str()))
                .await?,
        );
        asr_provider = Some(preset);
        asr_model = ctx.config.tools.whisper_model_path.clone();
    }

    let transcript = transcript.ok_or_else(no_transcript_asr_not_configured)?;
    let metadata = CacheMetadata {
        app_version: VERSION.to_string(),
        asr_provider,
        asr_model,
        video_id: video.id.clone(),
        platform: video.platform.as_str().to_string(),
        source: match transcript.source {
            crate::models::TranscriptSource::Subtitle => "subtitle".into(),
            crate::models::TranscriptSource::Asr => "asr".into(),
        },
        created_at: Utc::now().to_rfc3339(),
    };

    std::fs::create_dir_all(item_cache).ok();
    save_cache(item_cache, &video, &transcript, &metadata)?;
    write_job_manifest(item_cache, key, "done")?;

    mgr.emit(JobProgress::Done {
        media_key: key.to_string(),
    });

    Ok(make_analyze_result(&video, &transcript, item_cache, key))
}

pub async fn transcribe_local(
    ctx: &PipelineContext,
    path: &Path,
) -> Result<AnalyzeResult, AvrError> {
    let path_buf = std::fs::canonicalize(path).map_err(|e| {
        AvrError::new(
            crate::ErrorCode::IoError,
            e.to_string(),
            "检查媒体文件路径。",
        )
    })?;
    let path = path_buf.as_path();
    let ffmpeg = FfmpegContext::resolve(&ctx.root, &ctx.config)?;
    let duration = Some(probe_duration(&ffmpeg, path).await?);
    let key = media_key(
        &MediaSource::LocalFile {
            path: path.to_path_buf(),
        },
        duration,
    )?;
    let key = processing_key(ctx, &key)?;
    let base_cache = effective_cache_dir(&ctx.root, &ctx.config);
    let item_cache = base_cache.join(&key);

    let mgr = shared_job_manager();
    let path_clone = path.to_path_buf();
    let ctx_clone = ctx.clone();
    let item_cache_run = item_cache.clone();
    let key_run = key.clone();

    mgr.run_once(&key, &item_cache, || async move {
        run_local_pipeline(&ctx_clone, &path_clone, &item_cache_run, &key_run)
            .await
            .map(|_| ())
    })
    .await?;

    build_result_from_cache(&item_cache, &key)
}

async fn run_local_pipeline(
    ctx: &PipelineContext,
    path: &Path,
    item_cache: &Path,
    key: &str,
) -> Result<AnalyzeResult, AvrError> {
    if let Some((mut video, trans, meta)) = load_cached_transcript(item_cache)? {
        // 内容身份不随移动变化；视觉源必须绑定这次实际输入的位置。
        video.url = path.display().to_string();
        video.webpage_url = video.url.clone();
        video.title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        video.platform = crate::models::Platform::Local;
        save_cache(item_cache, &video, &trans, &meta)?;
        return Ok(make_analyze_result(&video, &trans, item_cache, key));
    }

    let ffmpeg = FfmpegContext::resolve(&ctx.root, &ctx.config)?;
    let duration = probe_duration(&ffmpeg, path).await.unwrap_or(0.0);
    let audio_path = item_cache.join("audio.wav");
    std::fs::create_dir_all(item_cache).ok();
    extract_audio_wav(&ffmpeg, path, &audio_path).await?;

    let asr = AsrService::new(ctx.root.clone(), ctx.config.clone());
    let preset = asr.active_preset(None, None);
    if !asr.is_ready(&preset) {
        return Err(no_transcript_asr_not_configured());
    }
    let transcript = asr.transcribe(&audio_path, None, None).await?;

    let video = crate::models::VideoInfo {
        id: key.to_string(),
        title: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "本地文件".into()),
        url: path.display().to_string(),
        duration,
        platform: crate::models::Platform::Local,
        webpage_url: path.display().to_string(),
    };

    let metadata = CacheMetadata {
        app_version: VERSION.to_string(),
        asr_provider: Some(preset),
        asr_model: ctx.config.tools.whisper_model_path.clone(),
        video_id: key.to_string(),
        platform: "local".into(),
        source: "asr".into(),
        created_at: Utc::now().to_rfc3339(),
    };

    save_cache(item_cache, &video, &transcript, &metadata)?;
    write_job_manifest(item_cache, key, "done")?;
    Ok(make_analyze_result(&video, &transcript, item_cache, key))
}

fn build_result_from_cache(item_cache: &Path, key: &str) -> Result<AnalyzeResult, AvrError> {
    let (video, trans, _) = load_cached_transcript(item_cache)?.ok_or_else(|| {
        AvrError::new(
            crate::error::ErrorCode::CacheError,
            "缓存不存在",
            "重新执行分析。",
        )
    })?;
    Ok(make_analyze_result(&video, &trans, item_cache, key))
}

fn make_analyze_result(
    video: &crate::models::VideoInfo,
    transcript: &Transcript,
    item_cache: &Path,
    key: &str,
) -> AnalyzeResult {
    let text_path = item_cache.join("transcript.txt");
    let path_str = text_path.display().to_string();
    AnalyzeResult {
        video: video.clone(),
        transcript: Some(transcript.clone()),
        transcript_path: path_str.clone(),
        transcript_uri: to_file_uri(&text_path),
        cache_dir: item_cache.display().to_string(),
        media_key: key.to_string(),
        truncated: transcript.full_text().len() > TRANSCRIPT_INLINE_LIMIT,
    }
}

fn write_job_manifest(cache_dir: &Path, key: &str, stage: &str) -> Result<(), AvrError> {
    crate::jobs::write_manifest(
        cache_dir,
        &JobManifest {
            media_key: key.to_string(),
            stage: stage.to_string(),
            updated_at: Utc::now().to_rfc3339(),
        },
    )
}

impl Clone for PipelineContext {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            config: self.config.clone(),
        }
    }
}

pub fn compact_mcp_payload(result: &AnalyzeResult, max_inline: usize) -> serde_json::Value {
    let trans = result.transcript.as_ref();
    let sample = trans
        .map(|t| t.segments.iter().take(5).cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let text = trans.map(|t| t.full_text()).unwrap_or_default();
    let truncated = text.chars().count() > max_inline;
    let inline = if truncated {
        format!("{}...", text.chars().take(max_inline).collect::<String>())
    } else {
        text
    };
    serde_json::json!({
        "media_key": result.media_key,
        "title": result.video.title,
        "duration": result.video.duration,
        "platform": result.video.platform,
        "video_id": result.video.id,
        "source": trans.map(|t| match t.source { crate::models::TranscriptSource::Subtitle => "subtitle", crate::models::TranscriptSource::Asr => "asr" }),
        "language": trans.map(|t| t.language.clone()),
        "segment_count": trans.map(|t| t.segments.len()).unwrap_or(0),
        "transcript_path": result.transcript_path,
        "transcript_uri": result.transcript_uri,
        "text_file": result.transcript_path,
        "cache_dir": result.cache_dir,
        "truncated": truncated,
        "sample_segments": sample,
        "has_video": true,
        "text": inline,
    })
}

pub fn full_mcp_payload(result: &AnalyzeResult) -> serde_json::Value {
    let trans = result.transcript.as_ref();
    serde_json::json!({
        "title": result.video.title,
        "platform": result.video.platform,
        "duration": result.video.duration,
        "video_id": result.video.id,
        "source": trans.map(|t| match t.source { crate::models::TranscriptSource::Subtitle => "subtitle", crate::models::TranscriptSource::Asr => "asr" }),
        "language": trans.map(|t| t.language.clone()),
        "truncated": result.truncated,
        "transcript_path": result.transcript_path,
        "transcript_uri": result.transcript_uri,
        "text_file": result.transcript_path,
        "cache_dir": result.cache_dir,
        "media_key": result.media_key,
        "segments": trans.map(|t| t.segments.clone()),
        "text": trans.map(|t| format_inline_text(t, result.truncated)),
    })
}

fn processing_key(ctx: &PipelineContext, media: &str) -> Result<String, AvrError> {
    let fingerprint = crate::asr::engine::processing_fingerprint(&ctx.root, &ctx.config)?;
    Ok(format!("{media}-asr-{}", &fingerprint[..24]))
}
