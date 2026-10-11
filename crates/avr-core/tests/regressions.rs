use avr_core::ffmpeg::MediaProbe;
use avr_core::frames::{clamp_automatic_times, sampling_times, validate_times};
use avr_core::media::{media_key, valid_media_key, MediaSource};
use avr_core::timeline::{parse_srt, parse_vtt};

#[test]
fn subtitle_repeated_speech_is_preserved() {
    let lf = "1\n00:00:00,000 --> 00:00:01,000\n重复\n\n2\n00:01:00,000 --> 00:01:01,000\n重复\n";
    let a = parse_srt(lf, "zh");
    assert_eq!(a.segments.len(), 2);
    assert_eq!(a.segments[0].text, "重复");
    assert_eq!(a.segments[1].text, "重复");
}

#[test]
fn vtt_parses_distinct_cues() {
    let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\n第一句\n\n00:01:00.000 --> 00:01:01.000\n第二句\n";
    let parsed = parse_vtt(vtt, "zh");
    assert_eq!(parsed.segments.len(), 2);
    assert_eq!(parsed.segments[0].text, "第一句");
    assert_eq!(parsed.segments[1].text, "第二句");
}

#[test]
fn url_identity_uses_bv_and_strips_tracking() {
    let key = |s: &str| media_key(&MediaSource::Url { url: s.into() }, None).unwrap();
    assert_eq!(
        key("https://www.bilibili.com/video/BV1AbCd?p=1"),
        key("https://www.bilibili.com/video/BV1AbCd?p=2")
    );
    assert_eq!(
        key("https://www.bilibili.com/video/BV1AbCd"),
        key("https://www.bilibili.com/video/BV1aBcD")
    );
    assert_eq!(
        key("https://www.bilibili.com/video/BV1AbCd"),
        key("https://www.bilibili.com/video/BV1AbCd?utm_source=x")
    );
}

#[test]
fn media_key_rejects_path_segments() {
    assert!(valid_media_key("abc-123"));
    assert!(!valid_media_key(""));
    assert!(!valid_media_key("."));
    assert!(!valid_media_key(".."));
    assert!(!valid_media_key("a/b"));
    assert!(!valid_media_key("a\\b"));
}

#[test]
fn model_bytes_change_content_fingerprint() {
    let t = tempfile::tempdir().unwrap();
    let model = t.path().join("model.bin");
    std::fs::write(&model, b"aaaa").unwrap();
    let a = avr_core::fingerprint::content_fingerprint(&model);
    std::fs::write(&model, b"bbbb").unwrap();
    let b = avr_core::fingerprint::content_fingerprint(&model);
    assert_ne!(a, b);
    assert!(a.is_some() && b.is_some());
}

#[test]
fn coverage_and_invalid_times() {
    let times = sampling_times(120.064, 2.0, 80, &[74.5, 74.5], &[]).unwrap();
    assert_eq!(times[0], 0.0);
    assert!(*times.last().unwrap() > 119.9);
    assert!(times.contains(&74.5));
    assert!(times.len() <= 80);
    assert_eq!(
        validate_times(&[2.0, 1.0, 1.0], 3.0).unwrap(),
        vec![1.0, 2.0]
    );
    for value in [f64::NAN, f64::INFINITY, -1.0, 3.0] {
        assert!(validate_times(&[value], 3.0).is_err());
    }
    assert!(sampling_times(1.0, 0.0, 2, &[], &[]).is_err());
    assert!(sampling_times(1.0, 1.0, 1, &[], &[]).is_err());
}

#[test]
fn automatic_samples_stop_before_the_last_frame() {
    let mut times = sampling_times(1.0, 2.0, 50, &[], &[]).unwrap();
    let probe = MediaProbe {
        duration: 1.0,
        width: 320,
        height: 240,
        frame_rate: 5.0,
        has_audio: false,
    };
    clamp_automatic_times(&mut times, &[], &probe);
    assert_eq!(times, vec![0.0, 0.8]);
}

#[test]
fn file_uri_roundtrips_encoded_path() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("中文 a.txt");
    std::fs::write(&p, b"x").unwrap();
    let uri = avr_core::paths::to_file_uri(&p);
    assert_eq!(
        url::Url::parse(&uri)
            .unwrap()
            .to_file_path()
            .unwrap()
            .canonicalize()
            .unwrap(),
        p.canonicalize().unwrap()
    );
    assert!(!uri.starts_with("file:////"));
    assert!(uri.contains("%20"));
    assert!(!uri.contains("中文"));
}
