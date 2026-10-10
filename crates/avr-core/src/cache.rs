//! 缓存读写。

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AvrError, ErrorCode};
use crate::jobs::atomic_write;
use crate::models::{CacheMetadata, Transcript, VideoInfo};
use crate::timeline::segments_to_srt;

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
        let content = fs::read_to_string(&video_path).ok()?;
        let data: serde_json::Value = serde_json::from_str(&content).ok()?;
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

pub fn load_cached_transcript(
    cache_dir: &Path,
) -> Result<Option<(VideoInfo, Transcript, CacheMetadata)>, AvrError> {
    let meta_path = cache_dir.join("metadata.json");
    let trans_path = cache_dir.join("transcript.json");
    if !meta_path.exists() || !trans_path.exists() {
        return Ok(None);
    }
    let meta: CacheMetadata =
        serde_json::from_str(&fs::read_to_string(&meta_path).map_err(io_err)?).map_err(json_err)?;
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
    AvrError::new(ErrorCode::IoError, e.to_string(), "检查磁盘权限。")
}
fn json_err(e: serde_json::Error) -> AvrError {
    AvrError::new(ErrorCode::CacheError, e.to_string(), "缓存文件损坏。")
}
