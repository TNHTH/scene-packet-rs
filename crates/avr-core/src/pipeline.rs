//! 视频分析流水线。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;

use crate::cache::{
    find_cache_by_url, load_cached_transcript, save_cache, AsrFingerprint, CACHE_SCHEMA,
};
use crate::config::{effective_cache_dir, load_config, AppConfig};
use crate::error::{no_transcript_asr_not_configured, unsupported_url, AvrError};
use crate::extract::platform::detect_platform;
use crate::jobs::{JobManager, JobManifest, JobProgress};
use crate::media::{media_key, MediaSource};
use crate::models::{AnalyzeResult, CacheMetadata, Transcript, TRANSCRIPT_INLINE_LIMIT};
use crate::paths::to_file_uri;
use crate::root::DataRoot;
use crate::tools::{RealTools, ToolId, Tools};
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
    let fp = asr_fingerprint(ctx, deps);

    if let Some(hit) = find_cache_by_url(&effective_cache_dir(&ctx.root, &ctx.config), url) {
        if load_cached_transcript(&hit, Some(&fp))?.is_some() {
            return build_result_from_cache(&hit, key);
        }
    }

    if let Some((video, trans, _)) = load_cached_transcript(item_cache, Some(&fp))? {
        return Ok(make_analyze_result(&video, &trans, item_cache, key));
    }

    stage("extracting", 20);
    let (video, info) = tools.extract_metadata(ctx, url).await?;

    stage("subtitles", 40);
    let subtitles = tools
        .fetch_subtitles(ctx, url, item_cache, &info)
        .await?
        .filter(|t| !t.segments.is_empty());

    let result = transcribe_and_save(
        ctx,
        deps,
        item_cache,
        key,
        &fp,
        &video,
        subtitles,
        |part| async move { tools.download_audio(ctx, url, &part).await },
    )
    .await?;

    mgr.emit(JobProgress::Done {
        media_key: key.to_string(),
    });
    Ok(result)
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
    let fp = asr_fingerprint(ctx, deps);
    if let Some((video, trans, _)) = load_cached_transcript(item_cache, Some(&fp))? {
        return Ok(make_analyze_result(&video, &trans, item_cache, key));
    }

    let tools = &deps.tools;
    let duration = tools.probe_duration(ctx, path).await.unwrap_or(0.0);
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

    transcribe_and_save(
        ctx,
        deps,
        item_cache,
        key,
        &fp,
        &video,
        None,
        |part| async move { tools.extract_audio(ctx, path, &part).await },
    )
    .await
}

/// URL 与本地共用的收尾阶段：
/// 没有可用字幕时检查 ASR → 获取音频（复用媒体缓存）→ 转写；然后写转写缓存与作业清单。
#[allow(clippy::too_many_arguments)]
async fn transcribe_and_save<F, Fut>(
    ctx: &PipelineContext,
    deps: &Deps,
    item_cache: &Path,
    key: &str,
    fp: &AsrFingerprint,
    video: &crate::models::VideoInfo,
    subtitles: Option<Transcript>,
    fetch_audio: F,
) -> Result<AnalyzeResult, AvrError>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<(), AvrError>>,
{
    let (transcript, asr_provider, asr_model, asr_version) = match subtitles {
        Some(t) => (t, "subtitle".to_string(), String::new(), String::new()),
        None => {
            let tools = &deps.tools;
            let platform =
                (video.platform != crate::models::Platform::Local).then(|| video.platform.as_str());
            let preset = tools
                .asr_preset(ctx, platform)
                .ok_or_else(no_transcript_asr_not_configured)?;
            deps.jobs.emit(JobProgress::Stage {
                media_key: key.to_string(),
                stage: "asr".into(),
                percent: 60,
            });
            let audio_path = ensure_audio(item_cache, fetch_audio).await?;
            let t = tools.transcribe(ctx, &audio_path, platform).await?;
            (t, preset, fp.model.clone(), fp.version.clone())
        }
    };

    let metadata = CacheMetadata {
        cache_schema: CACHE_SCHEMA,
        app_version: VERSION.to_string(),
        asr_provider: Some(asr_provider),
        asr_model,
        asr_version,
        video_id: video.id.clone(),
        platform: video.platform.as_str().to_string(),
        source: match transcript.source {
            crate::models::TranscriptSource::Subtitle => "subtitle".into(),
            crate::models::TranscriptSource::Asr => "asr".into(),
        },
        created_at: Utc::now().to_rfc3339(),
    };

    save_cache(item_cache, video, &transcript, &metadata)?;
    write_job_manifest(item_cache, key, "done")?;
    Ok(make_analyze_result(video, &transcript, item_cache, key))
}

/// 当前 ASR 配置的指纹（模型路径 + whisper 版本）。
fn asr_fingerprint(ctx: &PipelineContext, deps: &Deps) -> AsrFingerprint {
    AsrFingerprint {
        model: ctx.config.tools.whisper_model_path.clone(),
        version: deps.tools.version(ctx, ToolId::Whisper).unwrap_or_default(),
    }
}

/// 媒体缓存：`audio.wav` 已存在则直接复用（转写缓存失效时不必重新下载 / 抽取）；
/// 否则先写到 `audio-tmp.wav`，成功后原子改名，避免中断留下半截音频被误用。
async fn ensure_audio<F, Fut>(item_cache: &Path, fetch: F) -> Result<PathBuf, AvrError>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<(), AvrError>>,
{
    let audio = item_cache.join("audio.wav");
    if audio.is_file() {
        return Ok(audio);
    }
    std::fs::create_dir_all(item_cache).map_err(|e| AvrError::io(e, "检查缓存目录权限。"))?;
    let part = item_cache.join("audio-tmp.wav");
    fetch(part.clone()).await?;
    std::fs::rename(&part, &audio).map_err(|e| AvrError::io(e, "检查缓存目录权限。"))?;
    Ok(audio)
}

fn build_result_from_cache(item_cache: &Path, key: &str) -> Result<AnalyzeResult, AvrError> {
    let (video, trans, _) = load_cached_transcript(item_cache, None)?.ok_or_else(|| {
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

    /// P2-3：换 ASR 版本后转写缓存不命中、重新转写；媒体缓存（audio.wav）仍命中，不重新下载。
    #[tokio::test]
    async fn asr_version_change_invalidates_transcript_but_keeps_media() {
        use crate::tools::runtime::fake::FakeTools;
        use std::sync::atomic::Ordering;

        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = test_ctx(&tmp);
        let fake = Arc::new(FakeTools::default()); // 无字幕 → 走 ASR
        *fake.asr_version.lock() = "v1".into();
        let deps = Deps::new(Arc::new(JobManager::new())).with_tools(fake.clone());
        let src = Source::Url("https://www.bilibili.com/video/BV1fake".into());
        let opts = AnalyzeOptions::new();

        analyze(&ctx, &src, &opts, &deps).await.unwrap();
        analyze(&ctx, &src, &opts, &deps).await.unwrap();
        assert_eq!(
            fake.transcribe_calls.load(Ordering::SeqCst),
            1,
            "同版本命中"
        );

        *fake.asr_version.lock() = "v2".into();
        let r = analyze(&ctx, &src, &opts, &deps).await.unwrap();
        assert_eq!(
            fake.transcribe_calls.load(Ordering::SeqCst),
            2,
            "换版本重转"
        );
        assert_eq!(fake.audio_calls.load(Ordering::SeqCst), 1, "音频缓存仍命中");
        let meta = load_cached_transcript(std::path::Path::new(&r.cache_dir), None)
            .unwrap()
            .unwrap()
            .2;
        assert_eq!(meta.asr_version, "v2");
        assert_eq!(meta.cache_schema, CACHE_SCHEMA);
    }

    /// P2-3：旧缓存（metadata.json 无 cache_schema）不崩溃，视为未命中并重建。
    #[tokio::test]
    async fn legacy_cache_without_schema_is_a_miss() {
        use crate::tools::runtime::fake::FakeTools;
        use std::sync::atomic::Ordering;

        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = test_ctx(&tmp);
        let media = tmp.path().join("clip.mp4");
        std::fs::write(&media, b"fake").unwrap();
        let fake = Arc::new(FakeTools::default());
        let deps = Deps::new(Arc::new(JobManager::new())).with_tools(fake.clone());
        let src = Source::Local(media);
        let r = analyze(&ctx, &src, &AnalyzeOptions::new(), &deps)
            .await
            .unwrap();

        // 模拟旧版本写下的 metadata.json：去掉 cache_schema / asr_version。
        let dir = std::path::Path::new(&r.cache_dir);
        let meta_path = dir.join("metadata.json");
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.remove("cache_schema");
        obj.remove("asr_version");
        std::fs::write(&meta_path, v.to_string()).unwrap();

        assert!(load_cached_transcript(dir, None).unwrap().is_none());
        analyze(&ctx, &src, &AnalyzeOptions::new(), &deps)
            .await
            .unwrap();
        assert_eq!(fake.transcribe_calls.load(Ordering::SeqCst), 2);
        assert!(load_cached_transcript(dir, None).unwrap().is_some());
    }

    /// P2-2：共用收尾阶段——无字幕走 ASR 并复用已有音频；有字幕不触发 ASR 与音频获取。
    #[tokio::test]
    async fn transcribe_and_save_stage() {
        use crate::models::{Platform, TranscriptSource, VideoInfo};
        use crate::tools::runtime::fake::FakeTools;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = test_ctx(&tmp);
        let fake = Arc::new(FakeTools::default());
        let deps = Deps::new(Arc::new(JobManager::new())).with_tools(fake.clone());
        let fp = AsrFingerprint {
            model: "m".into(),
            version: "v".into(),
        };
        let video = VideoInfo {
            id: "k".into(),
            title: "t".into(),
            url: "u".into(),
            duration: 1.0,
            platform: Platform::Bilibili,
            webpage_url: "u".into(),
        };
        let fetches = AtomicUsize::new(0);
        let fetch = |part: PathBuf| {
            fetches.fetch_add(1, Ordering::SeqCst);
            async move { std::fs::write(part, b"RIFF").map_err(|e| AvrError::io(e, "")) }
        };

        // ASR 路径：获取音频 + 转写 + 写缓存（含指纹）。
        let dir = tmp.path().join("asr");
        let r = transcribe_and_save(&ctx, &deps, &dir, "k", &fp, &video, None, fetch)
            .await
            .unwrap();
        assert_eq!(r.transcript.unwrap().source, TranscriptSource::Asr);
        assert!(dir.join("audio.wav").is_file() && !dir.join("audio-tmp.wav").exists());
        let meta = load_cached_transcript(&dir, Some(&fp)).unwrap().unwrap().2;
        assert_eq!(
            (meta.asr_model.as_str(), meta.asr_version.as_str()),
            ("m", "v")
        );

        // 再跑一次：音频已存在，不再获取。
        transcribe_and_save(&ctx, &deps, &dir, "k", &fp, &video, None, fetch)
            .await
            .unwrap();
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        assert_eq!(fake.transcribe_calls.load(Ordering::SeqCst), 2);

        // 字幕路径：不转写、不取音频，元数据不带 ASR 指纹。
        let dir = tmp.path().join("sub");
        let sub = crate::models::Transcript {
            language: "zh".into(),
            source: TranscriptSource::Subtitle,
            segments: vec![],
        };
        transcribe_and_save(&ctx, &deps, &dir, "k", &fp, &video, Some(sub), fetch)
            .await
            .unwrap();
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        assert_eq!(fake.transcribe_calls.load(Ordering::SeqCst), 2);
        let meta = load_cached_transcript(&dir, None).unwrap().unwrap().2;
        assert_eq!(meta.source, "subtitle");
        assert!(meta.asr_version.is_empty());
    }
}
