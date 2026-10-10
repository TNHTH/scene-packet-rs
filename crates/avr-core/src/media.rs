//! 媒体来源与 media_key 规则。

use std::fs::File;
use std::sync::LazyLock;

use regex::Regex;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;
use xxhash_rust::xxh3::xxh3_64;

use crate::error::AvrError;
use crate::extract::platform::{detect_platform, Platform};
use crate::models::Platform as ModelPlatform;

const FINGERPRINT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaSource {
    Url { url: String },
    LocalFile { path: PathBuf },
}

impl MediaSource {
    pub fn display_label(&self) -> String {
        match self {
            Self::Url { url } => url.clone(),
            Self::LocalFile { path } => path.display().to_string(),
        }
    }
}

/// URL 归一化：剥 utm/spm 等追踪参数，保留平台 id。
pub fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    if let Ok(mut parsed) = Url::parse(&with_scheme) {
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .filter(|(k, _)| {
                let kl = k.to_lowercase();
                !kl.starts_with("utm_")
                    && kl != "spm_id_from"
                    && kl != "from"
                    && kl != "share_source"
            })
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        parsed.set_query(None);
        if !pairs.is_empty() {
            let mut qp = parsed.query_pairs_mut();
            for (k, v) in pairs {
                qp.append_pair(&k, &v);
            }
        }
        return parsed.to_string();
    }
    with_scheme
}

/// 从 URL 提取平台级 id（bilibili:BVxxx / douyin:xxx）。
pub fn platform_media_id(url: &str) -> Option<String> {
    let platform = detect_platform(url)?;
    let normalized = normalize_url(url);
    let lower = normalized.to_lowercase();

    match platform {
        Platform::Bilibili => {
            if let Some(bv) = regex_bv(&lower) {
                return Some(format!("bilibili:{bv}"));
            }
        }
        Platform::Douyin => {
            if let Some(id) = regex_douyin_id(&lower) {
                return Some(format!("douyin:{id}"));
            }
        }
    }
    None
}

static BV_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"bv[0-9a-z]+").expect("valid regex"));
static DOUYIN_VIDEO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/video/(\d+)").expect("valid regex"));
static DOUYIN_MODAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"modal_id=(\d+)").expect("valid regex"));

fn regex_bv(s: &str) -> Option<String> {
    BV_RE.find(s).map(|m| m.as_str().to_uppercase())
}

fn regex_douyin_id(s: &str) -> Option<String> {
    if let Some(cap) = DOUYIN_VIDEO_RE.captures(s) {
        return cap.get(1).map(|m| m.as_str().to_string());
    }
    DOUYIN_MODAL_RE
        .captures(s)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// 计算 media_key（缓存目录名）。
pub fn media_key(source: &MediaSource, duration: Option<f64>) -> Result<String, AvrError> {
    match source {
        MediaSource::Url { url } => {
            if let Some(pid) = platform_media_id(url) {
                let hash = xxh3_64(pid.as_bytes());
                Ok(format!("{}-{:016x}", pid.replace(':', "-"), hash))
            } else {
                let n = normalize_url(url);
                Ok(format!("url-{:016x}", xxh3_64(n.as_bytes())))
            }
        }
        MediaSource::LocalFile { path } => {
            let meta = std::fs::metadata(path).map_err(io_to_avr)?;
            let size = meta.len();
            let fp = file_fingerprint(path).map_err(io_to_avr)?;
            let dur = duration.unwrap_or(0.0);
            let seed = format!("{fp:x}:{size}:{dur:.3}");
            Ok(format!("local-{:016x}", xxh3_64(seed.as_bytes())))
        }
    }
}

fn file_fingerprint(path: &Path) -> Result<u64, std::io::Error> {
    let mut file = File::open(path)?;
    let mut buf = vec![0u8; FINGERPRINT_BYTES];
    let n = file.read(&mut buf)?;
    Ok(xxh3_64(&buf[..n]))
}

pub fn to_model_platform(p: Platform) -> ModelPlatform {
    match p {
        Platform::Bilibili => ModelPlatform::Bilibili,
        Platform::Douyin => ModelPlatform::Douyin,
    }
}

fn io_to_avr(e: std::io::Error) -> AvrError {
    AvrError::new(
        crate::error::ErrorCode::IoError,
        e.to_string(),
        "读取文件元数据失败。",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_with_utm_same_key() {
        let a = "https://www.bilibili.com/video/BV1test?utm_source=share";
        let b = "https://www.bilibili.com/video/BV1test";
        let ka = media_key(&MediaSource::Url { url: a.into() }, None).unwrap();
        let kb = media_key(&MediaSource::Url { url: b.into() }, None).unwrap();
        assert_eq!(ka, kb);
    }
}
