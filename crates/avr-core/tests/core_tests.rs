//! avr-core 单元测试。

#[cfg(test)]
mod tests {
    use avr_core::asr::select_preset;
    use avr_core::extract::platform::{detect_platform, Platform};
    use avr_core::media::{media_key, normalize_url, MediaSource};
    use avr_core::timeline::parse_vtt;

    #[test]
    fn test_bilibili_urls() {
        assert_eq!(
            detect_platform("https://www.bilibili.com/video/BV1xx"),
            Some(Platform::Bilibili)
        );
    }

    #[test]
    fn test_youtube_unsupported() {
        assert!(detect_platform("https://www.youtube.com/watch?v=x").is_none());
    }

    #[test]
    fn test_url_utm_normalized() {
        let a = normalize_url("https://www.bilibili.com/video/BV1test?utm_source=share");
        let b = normalize_url("https://www.bilibili.com/video/BV1test");
        let ka = media_key(&MediaSource::Url { url: a }, None).unwrap();
        let kb = media_key(&MediaSource::Url { url: b }, None).unwrap();
        assert_eq!(ka, kb);
    }

    #[test]
    fn test_select_provider_auto() {
        assert_eq!(
            select_preset("auto", None, Some("bilibili")),
            "whisper-cpp-cpu"
        );
        assert_eq!(
            select_preset("auto", Some("en"), None),
            "whisper-cpp-cublas"
        );
    }

    #[test]
    fn test_parse_vtt() {
        let vtt = include_str!("../../../tests/fixtures/sample.zh.vtt");
        let t = parse_vtt(vtt, "zh");
        assert_eq!(t.segments.len(), 2);
    }

    #[test]
    fn test_error_format_has_hint() {
        let e = avr_core::unsupported_url();
        let s = e.format_user();
        assert!(s.contains("UNSUPPORTED_URL"));
        assert!(s.contains("下一步"));
    }

    #[test]
    fn test_needs_first_run_no_pointer() {
        // 仅验证函数可调用；不修改真实 APPDATA
        let _ = avr_core::root::needs_first_run_setup(None);
    }
}
