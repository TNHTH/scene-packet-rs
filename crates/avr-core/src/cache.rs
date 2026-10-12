//! 缓存读写。

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AvrError, ErrorCode};
use crate::jobs::atomic_write;
use crate::models::{CacheMetadata, Transcript, VideoInfo};
use crate::timeline::segments_to_srt;

/// 缓存格式版本。metadata.json 中的 `cache_schema` 与之不一致时视为无缓存。
/// 2：新增 cache_schema / asr_version 字段。
pub const CACHE_SCHEMA: u32 = 2;

/// 当前 ASR 配置的指纹：模型与引擎版本。ASR 来源的转写缓存只有指纹一致才命中。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AsrFingerprint {
    pub model: String,
    pub version: String,
}

pub fn cache_dir_name(platform: &str, video_id: &str) -> String {
    format!("{platform}-{video_id}")
}

pub fn find_cache_by_url(base_cache: &Path, url: &str) -> Option<PathBuf> {
    if !base_cache.exists() {
        return None;
    }
    let normalized = url.trim();
    for entry in fs::read_dir(base_cache).into_iter().flatten().flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let video_path = path.join("video.json");
        if !video_path.exists() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&video_path) else {
            continue;
        };
        let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let u = data.get("url").and_then(|v| v.as_str());
        let w = data.get("webpage_url").and_then(|v| v.as_str());
        if (u == Some(normalized) || w == Some(normalized))
            && path.join("transcript.json").exists()
            && path.join("metadata.json").exists()
        {
            return Some(path);
        }
    }
    None
}

/// 读取转写缓存。
///
/// - `cache_schema` 不等于 `CACHE_SCHEMA`（含旧缓存缺字段）→ `Ok(None)`；
/// - `asr` 为 Some 且缓存来源是 ASR，模型或版本不一致 → `Ok(None)`；字幕来源不受影响；
/// - `asr` 为 None 时不检查 ASR 指纹（例如只读查看缓存）。
pub fn load_cached_transcript(
    cache_dir: &Path,
    asr: Option<&AsrFingerprint>,
) -> Result<Option<(VideoInfo, Transcript, CacheMetadata)>, AvrError> {
    let meta_path = cache_dir.join("metadata.json");
    let trans_path = cache_dir.join("transcript.json");
    if !meta_path.exists() || !trans_path.exists() {
        return Ok(None);
    }
    let meta: CacheMetadata =
        serde_json::from_str(&fs::read_to_string(&meta_path).map_err(io_err)?).map_err(json_err)?;
    if meta.cache_schema != CACHE_SCHEMA {
        return Ok(None);
    }
    if let Some(fp) = asr {
        if meta.source == "asr" && (meta.asr_model != fp.model || meta.asr_version != fp.version) {
            return Ok(None);
        }
    }
    let trans: Transcript = serde_json::from_str(&fs::read_to_string(&trans_path).map_err(io_err)?)
        .map_err(json_err)?;
    let video_path = cache_dir.join("video.json");
    let video = if video_path.exists() {
        serde_json::from_str(&fs::read_to_string(&video_path).map_err(io_err)?).map_err(json_err)?
    } else {
        VideoInfo {
            id: meta.video_id.clone(),
            title: String::new(),
            url: String::new(),
            duration: 0.0,
            // 旧缓存未知平台按 bilibili 兜底，保持向后兼容。
            platform: crate::models::Platform::parse(&meta.platform)
                .unwrap_or(crate::models::Platform::Bilibili),
            webpage_url: String::new(),
        }
    };
    Ok(Some((video, trans, meta)))
}

pub fn save_cache(
    cache_dir: &Path,
    video: &VideoInfo,
    transcript: &Transcript,
    metadata: &CacheMetadata,
) -> Result<PathBuf, AvrError> {
    fs::create_dir_all(cache_dir).map_err(io_err)?;
    atomic_write(&cache_dir.join("video.json"), video_json(video)?.as_bytes())?;
    atomic_write(
        &cache_dir.join("transcript.json"),
        transcript_json(transcript)?.as_bytes(),
    )?;
    atomic_write(
        &cache_dir.join("metadata.json"),
        metadata_json(metadata)?.as_bytes(),
    )?;
    atomic_write(
        &cache_dir.join("subtitle.srt"),
        segments_to_srt(transcript).as_bytes(),
    )?;
    let text_path = cache_dir.join("transcript.txt");
    atomic_write(&text_path, transcript.full_text().as_bytes())?;
    Ok(text_path)
}

fn video_json(v: &VideoInfo) -> Result<String, AvrError> {
    serde_json::to_string_pretty(v).map_err(json_err)
}
fn transcript_json(t: &Transcript) -> Result<String, AvrError> {
    serde_json::to_string_pretty(t).map_err(json_err)
}
fn metadata_json(m: &CacheMetadata) -> Result<String, AvrError> {
    serde_json::to_string_pretty(m).map_err(json_err)
}

fn io_err(e: std::io::Error) -> AvrError {
    AvrError::io(e, "检查磁盘权限。")
}
fn json_err(e: serde_json::Error) -> AvrError {
    AvrError::new(
        ErrorCode::CacheError,
        "缓存 JSON 解析失败",
        "缓存文件损坏。",
    )
    .with_source(e)
}
