//! 平台检测（零网络，1:1 移植 V1）。

use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Bilibili,
    Douyin,
}

static BILI_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"bilibili\.com/video/",
        r"b23\.tv/",
        r"bilibili\.com/bangumi/",
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});

static DOUYIN_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [r"douyin\.com", r"v\.douyin\.com", r"iesdouyin\.com"]
        .iter()
        .map(|p| Regex::new(p).unwrap())
        .collect()
});

pub fn detect_platform(url: &str) -> Option<Platform> {
    let lower = url.to_lowercase();
    let trimmed = lower.trim();
    let full = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    for pat in BILI_PATTERNS.iter() {
        if pat.is_match(&full) {
            return Some(Platform::Bilibili);
        }
    }
    for pat in DOUYIN_PATTERNS.iter() {
        if pat.is_match(&full) {
            return Some(Platform::Douyin);
        }
    }
    None
}

pub fn is_supported(url: &str) -> bool {
    detect_platform(url).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilibili_urls() {
        assert_eq!(
            detect_platform("https://www.bilibili.com/video/BV1xx"),
            Some(Platform::Bilibili)
        );
        assert_eq!(
            detect_platform("https://b23.tv/abc"),
            Some(Platform::Bilibili)
        );
    }

    #[test]
    fn douyin_urls() {
        assert_eq!(
            detect_platform("https://www.douyin.com/video/123"),
            Some(Platform::Douyin)
        );
        assert_eq!(
            detect_platform("https://v.douyin.com/abc"),
            Some(Platform::Douyin)
        );
    }

    #[test]
    fn youtube_unsupported() {
        assert_eq!(
            detect_platform("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            None
        );
    }
}
