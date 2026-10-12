//! 媒体来源与 media_key 规则。

use std::sync::LazyLock;

use regex::Regex;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;
use xxhash_rust::xxh3::xxh3_64;

use crate::error::AvrError;
use crate::extract::platform::{detect_platform, Platform};
use crate::models::Platform as ModelPlatform;

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

/// 经过白名单校验的 media_key，可安全地作为缓存目录名拼接路径。
///
/// 规则：1..=128 个字符，只含 ASCII 字母数字与 `-` `_` `.`，且不能以 `.` 开头
///（排除 `.`、`..` 与隐藏目录）。所有用 media_key 拼缓存路径的地方只接受这个类型。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MediaKey(String);

impl MediaKey {
    pub const MAX_LEN: usize = 128;

    pub fn parse(raw: &str) -> Result<Self, AvrError> {
        let ok = !raw.is_empty()
            && raw.len() <= Self::MAX_LEN
            && !raw.starts_with('.')
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if ok {
            Ok(Self(raw.to_string()))
        } else {
            Err(AvrError::new(
                crate::error::ErrorCode::CacheError,
                "media_key 非法",
                "使用 analyze 返回的 media_key。",
            ))
        }
    }

    /// 仅用于显示 / 日志 / JSON 输出；拼路径请用 [`MediaKey::dir`]。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MediaKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl MediaKey {
    /// 该媒体在缓存根 `cache_root` 下的目录。全仓库拼缓存子目录只走这里。
    pub fn dir(&self, cache_root: &Path) -> PathBuf {
        cache_root.join(&self.0)
    }
}

/// `media_key` 能否作为缓存目录名。拒绝空值、`.`、`..` 和路径分隔符。
pub fn valid_media_key(value: &str) -> bool {
    MediaKey::parse(value).is_ok()
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
            // 完整内容身份；时长探测失败或同文件复制不应改变缓存身份。
            let _ = duration;
            Ok(format!("local-{}", crate::storage::sha256(path)?))
        }
    }
}

pub fn to_model_platform(p: Platform) -> ModelPlatform {
    match p {
        Platform::Bilibili => ModelPlatform::Bilibili,
        Platform::Douyin => ModelPlatform::Douyin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_key_rejects_traversal() {
        for bad in [
            "",
            ".",
            "..",
            "../etc",
            "..\\x",
            "a/b",
            "a\\b",
            "/etc/passwd",
            "C:\\x",
            ".hidden",
            "a b",
            "键",
            "a\0b",
        ] {
            assert!(MediaKey::parse(bad).is_err(), "{bad:?} 应被拒绝");
        }
        assert!(MediaKey::parse(&"a".repeat(129)).is_err());
        for good in ["local-0123abcd", "bilibili-BV1xx-00ff", "url-ab.cd_ef"] {
            assert_eq!(MediaKey::parse(good).unwrap().as_str(), good);
        }
    }

    #[test]
    fn url_with_utm_same_key() {
        let a = "https://www.bilibili.com/video/BV1test?utm_source=share";
        let b = "https://www.bilibili.com/video/BV1test";
        let ka = media_key(&MediaSource::Url { url: a.into() }, None).unwrap();
        let kb = media_key(&MediaSource::Url { url: b.into() }, None).unwrap();
        assert_eq!(ka, kb);
    }
}
