//! 外部工具抽象：流水线只依赖 `Tools` trait，真实实现调用 ffmpeg / yt-dlp / whisper，
//! 测试注入假实现，不需要安装任何外部程序。

use std::path::Path;

use async_trait::async_trait;
use serde_json::Value;

use crate::asr::AsrService;
use crate::error::AvrError;
use crate::extract::ytdlp::YtDlpExtractor;
use crate::ffmpeg::{extract_audio_wav, probe_duration, FfmpegContext};
use crate::models::{Transcript, VideoInfo};
use crate::pipeline::PipelineContext;
use crate::tools::registry::ToolRegistry;
use crate::tools::ToolId;

#[async_trait]
pub trait Tools: Send + Sync {
    /// 媒体时长（秒）。
    async fn probe_duration(&self, ctx: &PipelineContext, input: &Path) -> Result<f64, AvrError>;
    /// 从本地媒体抽取 16k 单声道 wav。
    async fn extract_audio(
        &self,
        ctx: &PipelineContext,
        input: &Path,
        out_wav: &Path,
    ) -> Result<(), AvrError>;
    /// 拉取在线视频元数据，返回解析结果与原始 JSON。
    async fn extract_metadata(
        &self,
        ctx: &PipelineContext,
        url: &str,
    ) -> Result<(VideoInfo, Value), AvrError>;
    /// 下载并解析字幕；无字幕返回 `Ok(None)`。
    async fn fetch_subtitles(
        &self,
        ctx: &PipelineContext,
        url: &str,
        dest_dir: &Path,
        info: &Value,
    ) -> Result<Option<Transcript>, AvrError>;
    /// 下载在线视频音频为 wav。
    async fn download_audio(
        &self,
        ctx: &PipelineContext,
        url: &str,
        dest_wav: &Path,
    ) -> Result<(), AvrError>;
    /// 当前可用的 ASR 预设；未就绪返回 None。
    fn asr_preset(&self, ctx: &PipelineContext, platform: Option<&str>) -> Option<String>;
    /// 工具版本指纹，用于缓存失效判断；工具不可用时返回 None。
    fn version(&self, ctx: &PipelineContext, id: ToolId) -> Option<String>;
    /// 语音识别。
    async fn transcribe(
        &self,
        ctx: &PipelineContext,
        audio: &Path,
        platform: Option<&str>,
    ) -> Result<Transcript, AvrError>;
}

/// 调用真实外部程序的实现。
#[derive(Debug, Clone, Copy, Default)]
pub struct RealTools;

#[async_trait]
impl Tools for RealTools {
    async fn probe_duration(&self, ctx: &PipelineContext, input: &Path) -> Result<f64, AvrError> {
        let ff = FfmpegContext::resolve(&ctx.root, &ctx.config)?;
        probe_duration(&ff, input).await
    }

    async fn extract_audio(
        &self,
        ctx: &PipelineContext,
        input: &Path,
        out_wav: &Path,
    ) -> Result<(), AvrError> {
        let ff = FfmpegContext::resolve(&ctx.root, &ctx.config)?;
        extract_audio_wav(&ff, input, out_wav).await.map(|_| ())
    }

    async fn extract_metadata(
        &self,
        ctx: &PipelineContext,
        url: &str,
    ) -> Result<(VideoInfo, Value), AvrError> {
        YtDlpExtractor::new(&ctx.root, &ctx.config)?
            .extract_metadata_with_info(url)
            .await
    }

    async fn fetch_subtitles(
        &self,
        ctx: &PipelineContext,
        url: &str,
        dest_dir: &Path,
        info: &Value,
    ) -> Result<Option<Transcript>, AvrError> {
        YtDlpExtractor::new(&ctx.root, &ctx.config)?
            .fetch_subtitles_with_info(url, dest_dir, info)
            .await
    }

    async fn download_audio(
        &self,
        ctx: &PipelineContext,
        url: &str,
        dest_wav: &Path,
    ) -> Result<(), AvrError> {
        let got = YtDlpExtractor::new(&ctx.root, &ctx.config)?
            .download_audio(url, dest_wav)
            .await?;
        // yt-dlp 可能按自己的规则命名产物；统一落到调用方要求的路径。
        if got != dest_wav {
            std::fs::rename(&got, dest_wav).map_err(|e| AvrError::io(e, "检查缓存目录权限。"))?;
        }
        Ok(())
    }

    fn asr_preset(&self, ctx: &PipelineContext, platform: Option<&str>) -> Option<String> {
        let asr = AsrService::new(ctx.root.clone(), ctx.config.clone());
        let preset = asr.active_preset(None, platform);
        asr.is_ready(&preset).then_some(preset)
    }

    /// 以可执行文件的大小与修改时间作为版本指纹：升级 / 替换二进制即失效，
    /// 且不需要在异步上下文里同步拉起子进程。
    fn version(&self, ctx: &PipelineContext, id: ToolId) -> Option<String> {
        let path = ToolRegistry::new(&ctx.root, &ctx.config)
            .tool_path(id)
            .ok()?;
        let meta = std::fs::metadata(&path).ok()?;
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(format!("{}:{mtime}", meta.len()))
    }

    async fn transcribe(
        &self,
        ctx: &PipelineContext,
        audio: &Path,
        platform: Option<&str>,
    ) -> Result<Transcript, AvrError> {
        AsrService::new(ctx.root.clone(), ctx.config.clone())
            .transcribe(audio, None, platform)
            .await
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! 测试用假实现：记录调用次数，按配置返回字幕或 ASR 结果。

    use super::*;
    use crate::models::{Platform, TranscriptSegment, TranscriptSource};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    pub(crate) struct FakeTools {
        /// true 时在线视频有字幕；false 时走 ASR。
        pub with_subtitles: bool,
        pub metadata_calls: AtomicUsize,
        pub transcribe_calls: AtomicUsize,
        /// download_audio / extract_audio 调用次数（媒体获取）。
        pub audio_calls: AtomicUsize,
        /// whisper 版本，测试可在两次调用之间修改。
        pub asr_version: parking_lot::Mutex<String>,
    }

    fn transcript(source: TranscriptSource) -> Transcript {
        Transcript {
            language: "zh".into(),
            source,
            segments: vec![TranscriptSegment {
                start: 0.0,
                end: 1.5,
                text: "你好世界".into(),
            }],
        }
    }

    #[async_trait]
    impl Tools for FakeTools {
        async fn probe_duration(&self, _: &PipelineContext, _: &Path) -> Result<f64, AvrError> {
            Ok(12.5)
        }

        async fn extract_audio(
            &self,
            _: &PipelineContext,
            _: &Path,
            out_wav: &Path,
        ) -> Result<(), AvrError> {
            self.audio_calls.fetch_add(1, Ordering::SeqCst);
            std::fs::write(out_wav, b"RIFF").map_err(|e| AvrError::io(e, ""))
        }

        async fn extract_metadata(
            &self,
            _: &PipelineContext,
            url: &str,
        ) -> Result<(VideoInfo, Value), AvrError> {
            self.metadata_calls.fetch_add(1, Ordering::SeqCst);
            let video = VideoInfo {
                id: "BV1fake".into(),
                title: "假视频".into(),
                url: url.into(),
                duration: 12.5,
                platform: Platform::Bilibili,
                webpage_url: url.into(),
            };
            Ok((video, Value::Null))
        }

        async fn fetch_subtitles(
            &self,
            _: &PipelineContext,
            _: &str,
            _: &Path,
            _: &Value,
        ) -> Result<Option<Transcript>, AvrError> {
            Ok(self
                .with_subtitles
                .then(|| transcript(TranscriptSource::Subtitle)))
        }

        async fn download_audio(
            &self,
            _: &PipelineContext,
            _: &str,
            dest_wav: &Path,
        ) -> Result<(), AvrError> {
            self.audio_calls.fetch_add(1, Ordering::SeqCst);
            std::fs::write(dest_wav, b"RIFF").map_err(|e| AvrError::io(e, ""))
        }

        fn asr_preset(&self, _: &PipelineContext, _: Option<&str>) -> Option<String> {
            Some("fake-asr".into())
        }

        fn version(&self, _: &PipelineContext, id: ToolId) -> Option<String> {
            (id == ToolId::Whisper).then(|| self.asr_version.lock().clone())
        }

        async fn transcribe(
            &self,
            _: &PipelineContext,
            _: &Path,
            _: Option<&str>,
        ) -> Result<Transcript, AvrError> {
            self.transcribe_calls.fetch_add(1, Ordering::SeqCst);
            Ok(transcript(TranscriptSource::Asr))
        }
    }
}
