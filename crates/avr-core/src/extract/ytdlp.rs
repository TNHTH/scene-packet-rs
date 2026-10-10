//! yt-dlp 子进程适配（B站/抖音）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::config::AppConfig;
use crate::error::{cookies_required, extract_failed, unsupported_url, ytdlp_missing, AvrError};
use crate::extract::platform::{detect_platform, is_supported, Platform};
use crate::ffmpeg::FfmpegContext;
use crate::media::to_model_platform;
use crate::models::{Transcript, VideoInfo};
use crate::process::{run, ProcessRequest};
use crate::root::DataRoot;
use crate::timeline::{parse_srt, parse_vtt, pick_subtitle_lang};
use crate::tools::registry::ToolRegistry;

pub struct YtDlpExtractor {
    ytdlp: PathBuf,
    cookies_from_browser: String,
    cookies_file: String,
    user_agent: String,
    ffmpeg: Option<FfmpegContext>,
}

impl YtDlpExtractor {
    pub fn new(root: &DataRoot, cfg: &AppConfig) -> Result<Self, AvrError> {
        let reg = ToolRegistry::new(root, cfg);
        let ytdlp = reg.ytdlp_path()?;
        let ffmpeg = FfmpegContext::resolve(root, cfg).ok();
        Ok(Self {
            ytdlp,
            cookies_from_browser: cfg.extractor.cookies_from_browser.clone(),
            cookies_file: cfg.extractor.cookies_file.clone(),
            user_agent: cfg.extractor.user_agent.clone(),
            ffmpeg,
        })
    }

    pub fn supports(&self, url: &str) -> bool {
        is_supported(url)
    }

    pub async fn extract_metadata(&self, url: &str) -> Result<VideoInfo, AvrError> {
        if !is_supported(url) {
            return Err(unsupported_url());
        }
        let info = self.extract_info(url).await?;
        Ok(video_info_from_json(&info, url))
    }

    pub async fn extract_metadata_with_info(
        &self,
        url: &str,
    ) -> Result<(VideoInfo, Value), AvrError> {
        let info = self.extract_info(url).await?;
        Ok((video_info_from_json(&info, url), info))
    }

    /// 复用已拉取的 JSON，避免抖音重复请求触发限流。
    pub async fn fetch_subtitles_with_info(
        &self,
        url: &str,
        dest_dir: &Path,
        info: &Value,
    ) -> Result<Option<Transcript>, AvrError> {
        if !is_supported(url) {
            return Err(unsupported_url());
        }
        let subs = merge_subs(info);
        if subs.is_empty() {
            return Ok(None);
        }
        let langs = subs.keys().cloned().collect::<Vec<_>>();
        let lang = pick_subtitle_lang(&langs).unwrap_or_else(|| langs[0].clone());

        std::fs::create_dir_all(dest_dir).ok();
        let outtmpl = dest_dir.join("subtitle");

        let mut args = base_args(
            &self.cookies_from_browser,
            &self.cookies_file,
            &self.user_agent,
            self.ffmpeg.as_ref(),
        );
        args.extend([
            "--skip-download".into(),
            "--write-subs".into(),
            "--write-auto-subs".into(),
            "--sub-langs".into(),
            lang.clone(),
            "-o".into(),
            outtmpl.to_string_lossy().to_string(),
            url.into(),
        ]);

        let req = ProcessRequest {
            program: self.ytdlp.clone(),
            args,
            cwd: None,
            timeout: Some(Duration::from_secs(300)),
            expected_outputs: vec![],
        };
        if run(&req).await.is_err() {
            return Ok(None);
        }

        for ext in [".vtt", ".srt"] {
            for entry in std::fs::read_dir(dest_dir).into_iter().flatten().flatten() {
                let p = entry.path();
                if p.file_name()
                    .is_some_and(|n| n.to_string_lossy().contains("subtitle"))
                    && p.extension()
                        .map(|e| e == ext.trim_start_matches('.'))
                        .unwrap_or(false)
                {
                    let content = std::fs::read_to_string(&p).unwrap_or_default();
                    if ext == ".vtt" {
                        return Ok(Some(parse_vtt(&content, &lang)));
                    }
                    return Ok(Some(parse_srt(&content, &lang)));
                }
            }
        }
        Ok(None)
    }

    pub async fn fetch_subtitles(
        &self,
        url: &str,
        dest_dir: &Path,
    ) -> Result<Option<Transcript>, AvrError> {
        let info = self.extract_info(url).await?;
        self.fetch_subtitles_with_info(url, dest_dir, &info).await
    }

    pub async fn download_audio(&self, url: &str, dest_wav: &Path) -> Result<PathBuf, AvrError> {
        if !is_supported(url) {
            return Err(unsupported_url());
        }
        let ffmpeg = self
            .ffmpeg
            .as_ref()
            .ok_or_else(crate::error::ffmpeg_missing)?;
        if !ffmpeg.ffmpeg.exists() {
            return Err(crate::error::ffmpeg_missing());
        }

        std::fs::create_dir_all(dest_wav.parent().unwrap_or(Path::new("."))).ok();
        let out_base = dest_wav.with_extension("");

        let mut args = base_args(
            &self.cookies_from_browser,
            &self.cookies_file,
            &self.user_agent,
            self.ffmpeg.as_ref(),
        );
        args.extend([
            "-f".into(),
            "bestaudio/best".into(),
            "-o".into(),
            out_base.to_string_lossy().to_string(),
            "--extract-audio".into(),
            "--audio-format".into(),
            "wav".into(),
            url.into(),
        ]);

        let req = ProcessRequest {
            program: self.ytdlp.clone(),
            args,
            cwd: None,
            timeout: Some(Duration::from_secs(3600)),
            expected_outputs: vec![],
        };
        run(&req).await?;

        if dest_wav.exists() {
            return Ok(dest_wav.to_path_buf());
        }
        if let Some(parent) = dest_wav.parent() {
            for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
                let p = entry.path();
                if p.extension().map(|e| e == "wav").unwrap_or(false) {
                    return Ok(p);
                }
            }
        }
        Err(extract_failed())
    }

    pub async fn download_section(
        &self,
        url: &str,
        start: f64,
        end: f64,
        output: &Path,
    ) -> Result<PathBuf, AvrError> {
        let section = format!("*{:.3}-{:.3}", start, end);
        let mut args = base_args(
            &self.cookies_from_browser,
            &self.cookies_file,
            &self.user_agent,
            self.ffmpeg.as_ref(),
        );
        args.extend([
            "--download-sections".into(),
            section,
            "-f".into(),
            "best[height<=480]/best".into(),
            "-o".into(),
            output.display().to_string(),
            url.into(),
        ]);
        let req = ProcessRequest {
            program: self.ytdlp.clone(),
            args,
            cwd: None,
            timeout: Some(Duration::from_secs(600)),
            expected_outputs: vec![output.to_path_buf()],
        };
        run(&req).await?;
        Ok(output.to_path_buf())
    }

    async fn extract_info(&self, url: &str) -> Result<Value, AvrError> {
        if self.ytdlp.as_os_str().is_empty() {
            return Err(ytdlp_missing());
        }
        let mut args = base_args(
            &self.cookies_from_browser,
            &self.cookies_file,
            &self.user_agent,
            self.ffmpeg.as_ref(),
        );
        args.extend(["--dump-json".into(), "--no-download".into(), url.into()]);
        let req = ProcessRequest {
            program: self.ytdlp.clone(),
            args,
            cwd: None,
            timeout: Some(Duration::from_secs(120)),
            expected_outputs: vec![],
        };
        match run(&req).await {
            Ok(out) => {
                if let Ok(json) = serde_json::from_str::<Value>(&out.stdout) {
                    if json.get("id").is_some() || json.get("display_id").is_some() {
                        return Ok(json);
                    }
                }
                let lower = format!("{} {}", out.stderr, out.stdout).to_lowercase();
                if detect_platform(url) == Some(Platform::Douyin)
                    && (lower.contains("fresh cookies")
                        || lower.contains("login")
                        || lower.contains("403 forbidden"))
                {
                    return Err(cookies_required());
                }
                Err(extract_failed())
            }
            Err(e) => {
                let msg = e.message.to_lowercase();
                if detect_platform(url) == Some(Platform::Douyin)
                    && (msg.contains("fresh cookies") || msg.contains("403 forbidden"))
                {
                    return Err(cookies_required());
                }
                Err(extract_failed())
            }
        }
    }
}

fn base_args(
    cookies_browser: &str,
    cookies_file: &str,
    user_agent: &str,
    ffmpeg: Option<&FfmpegContext>,
) -> Vec<String> {
    let mut args = vec![
        "--quiet".into(),
        "--no-warnings".into(),
        "--no-progress".into(),
    ];
    if let Some(ff) = ffmpeg {
        if let Some(dir) = ff.ffmpeg.parent() {
            args.push("--ffmpeg-location".into());
            args.push(dir.display().to_string());
        }
    }
    if !user_agent.is_empty() {
        args.push("--user-agent".into());
        args.push(user_agent.into());
    }
    if !cookies_file.is_empty() {
        args.push("--cookies".into());
        args.push(cookies_file.into());
    } else if cookies_browser != "none" && !cookies_browser.is_empty() {
        args.push("--cookies-from-browser".into());
        args.push(cookies_browser.into());
    }
    args
}

fn video_info_from_json(info: &Value, url: &str) -> VideoInfo {
    let platform = detect_platform(url).unwrap_or(Platform::Bilibili);
    let id = info
        .get("id")
        .or_else(|| info.get("display_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    VideoInfo {
        id,
        title: info
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("未命名视频")
            .to_string(),
        url: url.to_string(),
        duration: info.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0),
        platform: to_model_platform(platform),
        webpage_url: info
            .get("webpage_url")
            .and_then(|v| v.as_str())
            .unwrap_or(url)
            .to_string(),
    }
}

fn merge_subs(info: &Value) -> std::collections::HashMap<String, Value> {
    let mut merged = std::collections::HashMap::new();
    if let Some(auto) = info.get("automatic_captions").and_then(|v| v.as_object()) {
        for (k, v) in auto {
            merged.insert(k.clone(), v.clone());
        }
    }
    if let Some(subs) = info.get("subtitles").and_then(|v| v.as_object()) {
        for (k, v) in subs {
            merged.insert(k.clone(), v.clone());
        }
    }
    merged
}
