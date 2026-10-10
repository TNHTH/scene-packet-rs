//! 视频分析流水线。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;

use crate::cache::{find_cache_by_url, load_cached_transcript, save_cache};
use crate::config::{effective_cache_dir, load_config, AppConfig};
use crate::error::{no_transcript_asr_not_configured, unsupported_url, AvrError};
use crate::extract::platform::detect_platform;
use crate::jobs::{JobManager, JobManifest, JobProgress};
use crate::media::{media_key, MediaSource};
use crate::models::{AnalyzeResult, CacheMetadata, Transcript, TRANSCRIPT_INLINE_LIMIT};
use crate::paths::to_file_uri;
use crate::root::DataRoot;
use crate::tools::{RealTools, Tools};
use crate::VERSION;

pub struct PipelineContext {
    pub root: DataRoot,
    pub config: AppConfig,
}

impl PipelineContext {
    pub fn load(root: DataRoot) -> Result<Self, AvrError> {
        root.ensure_layout()?;
        let config = load_config(&root)?;
        Ok(Self { root, config })
    }
}

/// 待分析的媒体来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// B站 / 抖音等在线视频链接。
    Url(String),
    /// 本地音视频文件。
    Local(PathBuf),
}

impl Source {
    /// 按输入字符串推断来源：`http://` / `https://` 开头视为 URL，其余视为本地路径。
    pub fn parse(input: &str) -> Self {
        if input.starts_with("http://") || input.starts_with("https://") {
            Self::Url(input.to_string())
        } else {
            Self::Local(PathBuf::from(input))
        }
    }
}

/// 分析选项。字段私有、只加不改：通过 `AnalyzeOptions::default()` 与 builder 方法构造。
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AnalyzeOptions {}

impl AnalyzeOptions {
    pub fn new() -> Self {
        Self::default()
    }
}

/// 流水线依赖，由调用方构造并注入。字段私有、只加不改：构造只走 `Deps::new`。
#[derive(Clone)]
pub struct Deps {
    jobs: Arc<JobManager>,
    tools: Arc<dyn Tools>,
}

impl Deps {
    /// 默认使用真实外部工具（`RealTools`）。
    pub fn new(jobs: Arc<JobManager>) -> Self {
        Self {
            jobs,
            tools: Arc::new(RealTools),
        }
    }

    /// 替换外部工具实现（测试或自定义后端）。
    pub fn with_tools(mut self, tools: Arc<dyn Tools>) -> Self {
        self.tools = tools;
        self
    }

    pub fn jobs(&self) -> &Arc<JobManager> {
        &self.jobs
    }

    pub fn tools(&self) -> &Arc<dyn Tools> {
        &self.tools
    }

    /// 测试用：独立的 JobManager + 默认假工具。
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self::new(Arc::new(JobManager::new()))
            .with_tools(Arc::new(crate::tools::runtime::fake::FakeTools::default()))
    }
}

/// 统一分析入口：GUI / CLI / MCP 都应调用这里。
pub async fn analyze(
    ctx: &PipelineContext,
    src: &Source,
    _opts: &AnalyzeOptions,
    deps: &Deps,
) -> Result<AnalyzeResult, AvrError> {
    match src {
        Source::Url(url) => analyze_url_with(ctx, url, deps).await,
        Source::Local(path) => transcribe_local_with(ctx, path, deps).await,
    }
}


async fn analyze_url_with(
    ctx: &PipelineContext,
    url: &str,
    deps: &Deps,
) -> Result<AnalyzeResult, AvrError> {
    if detect_platform(url).is_none() {
        return Err(unsupported_url());
    }

    let base_cache = effective_cache_dir(&ctx.root, &ctx.config);
    let key = media_key(
        &MediaSource::Url {
            url: url.to_string(),
        },
        None,
    )?;
    let item_cache = base_cache.join(&key);

    let url_clone = url.to_string();
    let ctx_clone = ctx.clone();
    let deps_clone = deps.clone();
    let item_cache_run = item_cache.clone();
    let key_run = key.clone();

    deps.jobs
        .run_once(&key, &item_cache, || async move {
            run_url_pipeline(
                &ctx_clone,
                &deps_clone,
                &url_clone,
                &item_cache_run,
                &key_run,
            )
            .await
            .map(|_| ())
        })
        .await?;

    build_result_from_cache(&item_cache, &key)
}

async fn run_url_pipeline(
    ctx: &PipelineContext,
    deps: &Deps,
    url: &str,
    item_cache: &Path,
    key: &str,
) -> Result<AnalyzeResult, AvrError> {
    let mgr = &deps.jobs;
    let tools = &deps.tools;
    let stage = |stage: &str, percent: u8| {
        mgr.emit(JobProgress::Stage {
            media_key: key.to_string(),
            stage: stage.into(),
            percent,
        })
    };
    stage("checking_cache", 5);

    if let Some(hit) = find_cache_by_url(&effective_cache_dir(&ctx.root, &ctx.config), url) {
        if load_cached_transcript(&hit)?.is_some() {
            return build_result_from_cache(&hit, key);
        }
    }

    if let Some((video, trans, _)) = load_cached_transcript(item_cache)? {
        return Ok(make_analyze_result(&video, &trans, item_cache, key));
    }

    stage("extracting", 20);
    let (video, info) = tools.extract_metadata(ctx, url).await?;

    stage("subtitles", 40);
    let subtitles = tools
        .fetch_subtitles(ctx, url, item_cache, &info)
        .await?
        .filter(|t| !t.segments.is_empty());

    let (transcript, asr_provider, asr_model) = match subtitles {
        Some(t) => (t, "subtitle".to_string(), String::new()),
        None => {
            let platform = Some(video.platform.as_str());
            let preset = tools
                .asr_preset(ctx, platform)
                .ok_or_else(no_transcript_asr_not_configured)?;
            stage("asr", 60);
            let audio_path = item_cache.join("audio.wav");
            tools.download_audio(ctx, url, &audio_path).await?;
            let t = tools.transcribe(ctx, &audio_path, platform).await?;
            (t, preset, ctx.config.tools.whisper_model_path.clone())
        }
    };

    let metadata = CacheMetadata {
        app_version: VERSION.to_string(),
        asr_provider: Some(asr_provider),
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

async fn transcribe_local_with(
    ctx: &PipelineContext,
    path: &Path,
    deps: &Deps,
) -> Result<AnalyzeResult, AvrError> {
    let duration = deps.tools.probe_duration(ctx, path).await.ok();
    let key = media_key(
        &MediaSource::LocalFile {
            path: path.to_path_buf(),
        },
        duration,
    )?;
    let item_cache = effective_cache_dir(&ctx.root, &ctx.config).join(&key);

    let path_clone = path.to_path_buf();
    let ctx_clone = ctx.clone();
    let deps_clone = deps.clone();
    let item_cache_run = item_cache.clone();
    let key_run = key.clone();

    deps.jobs
        .run_once(&key, &item_cache, || async move {
            run_local_pipeline(
                &ctx_clone,
                &deps_clone,
                &path_clone,
                &item_cache_run,
                &key_run,
            )
            .await
            .map(|_| ())
        })
        .await?;

    build_result_from_cache(&item_cache, &key)
}

async fn run_local_pipeline(
    ctx: &PipelineContext,
    deps: &Deps,
    path: &Path,
    item_cache: &Path,
    key: &str,
) -> Result<AnalyzeResult, AvrError> {
    if let Some((video, trans, _)) = load_cached_transcript(item_cache)? {
        return Ok(make_analyze_result(&video, &trans, item_cache, key));
    }

    let tools = &deps.tools;
    let duration = tools.probe_duration(ctx, path).await.unwrap_or(0.0);
    let audio_path = item_cache.join("audio.wav");
    std::fs::create_dir_all(item_cache).ok();
    tools.extract_audio(ctx, path, &audio_path).await?;

    let preset = tools
        .asr_preset(ctx, None)
        .ok_or_else(no_transcript_asr_not_configured)?;
    let transcript = tools.transcribe(ctx, &audio_path, None).await?;
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
        platform: crate::models::Platform::Local.as_str().into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_parse_distinguishes_url_and_local() {
        assert_eq!(
            Source::parse("https://www.bilibili.com/video/BV1x"),
            Source::Url("https://www.bilibili.com/video/BV1x".into())
        );
        assert_eq!(
            Source::parse("C:/videos/a.mp4"),
            Source::Local(PathBuf::from("C:/videos/a.mp4"))
        );
    }

    #[tokio::test]
    async fn analyze_rejects_unsupported_url() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = PipelineContext {
            root: DataRoot {
                path: tmp.path().to_path_buf(),
            },
            config: AppConfig::default(),
        };
        let src = Source::Url("https://www.youtube.com/watch?v=x".into());
        let r = analyze(&ctx, &src, &AnalyzeOptions::new(), &Deps::for_test()).await;
        assert!(matches!(r, Err(ref e) if e.code == crate::ErrorCode::UnsupportedUrl));
    }

    fn test_ctx(tmp: &tempfile::TempDir) -> PipelineContext {
        PipelineContext {
            root: DataRoot {
                path: tmp.path().to_path_buf(),
            },
            config: AppConfig::default(),
        }
    }

    /// P1-3：不装任何外部程序，用 FakeTools 跑通 URL 流水线（字幕路径）并验证缓存复用。
    #[tokio::test]
    async fn url_pipeline_runs_with_fake_tools_and_reuses_cache() {
        use crate::tools::runtime::fake::FakeTools;
        use std::sync::atomic::Ordering;

        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = test_ctx(&tmp);
        let fake = Arc::new(FakeTools {
            with_subtitles: true,
            ..Default::default()
        });
        let deps = Deps::new(Arc::new(JobManager::new())).with_tools(fake.clone());
        let src = Source::Url("https://www.bilibili.com/video/BV1fake".into());

        let r = analyze(&ctx, &src, &AnalyzeOptions::new(), &deps)
            .await
            .unwrap();
        assert_eq!(r.video.title, "假视频");
        let t = r.transcript.as_ref().unwrap();
        assert_eq!(t.source, crate::models::TranscriptSource::Subtitle);
        assert!(std::path::Path::new(&r.transcript_path).exists());

        let again = analyze(&ctx, &src, &AnalyzeOptions::new(), &deps)
            .await
            .unwrap();
        assert_eq!(again.media_key, r.media_key);
        assert_eq!(
            fake.metadata_calls.load(Ordering::SeqCst),
            1,
            "第二次命中缓存"
        );
        assert_eq!(fake.transcribe_calls.load(Ordering::SeqCst), 0);
    }

    /// P1-3：本地文件走 ASR 路径，平台标记为 Local。
    #[tokio::test]
    async fn local_pipeline_runs_asr_with_fake_tools() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = test_ctx(&tmp);
        let media = tmp.path().join("clip.mp4");
        std::fs::write(&media, b"fake").unwrap();
        let r = analyze(
            &ctx,
            &Source::Local(media),
            &AnalyzeOptions::new(),
            &Deps::for_test(),
        )
        .await
        .unwrap();
        assert_eq!(r.video.platform, crate::models::Platform::Local);
        assert_eq!(
            r.transcript.unwrap().source,
            crate::models::TranscriptSource::Asr
        );
    }
}
