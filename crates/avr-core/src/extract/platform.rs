//! 平台检测（零网络，1:1 移植 V1）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Bilibili,
    Douyin,
}

pub fn detect_platform(input: &str) -> Option<Platform> {
    let input = input.trim();
    let full = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let url = url::Url::parse(&full).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let belongs = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if belongs("b23.tv")
        || (belongs("bilibili.com")
            && (url.path().starts_with("/video/") || url.path().starts_with("/bangumi/")))
    {
        Some(Platform::Bilibili)
    } else if belongs("douyin.com") || belongs("iesdouyin.com") {
        Some(Platform::Douyin)
    } else {
        None
    }
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
