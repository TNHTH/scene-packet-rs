use avr_core::{
    config::AppConfig,
    frames::{sampling_times, validate_times},
    media::{media_key, MediaSource},
    root::DataRoot,
    timeline::{parse_srt, parse_vtt},
};
#[test]
fn subtitle_line_endings_and_repeated_speech_are_preserved() {
    let lf = "1\n00:00:00,000 --> 00:00:01,000\n重复\n\n2\n00:01:00,000 --> 00:01:01,000\n重复\n";
    let a = parse_srt(lf, "zh");
    let b = parse_srt(&format!("\u{feff}{}", lf.replace('\n', "\r\n")), "zh");
    assert_eq!(a, b);
    assert_eq!(a.segments.len(), 2);
    let vtt =
        "WEBVTT\n\nfirst\n00:00.000 --> 00:01.000\n重复\n\nsecond\n01:00.000 --> 01:01.000\n重复\n";
    assert_eq!(parse_vtt(vtt, "zh").segments, a.segments);
}
#[test]
fn url_identity_preserves_part_and_bv_case() {
    let key = |s: &str| media_key(&MediaSource::Url { url: s.into() }, None).unwrap();
    assert_ne!(
        key("https://www.bilibili.com/video/BV1AbCd?p=1"),
        key("https://www.bilibili.com/video/BV1AbCd?p=2")
    );
    assert_ne!(
        key("https://www.bilibili.com/video/BV1AbCd"),
        key("https://www.bilibili.com/video/BV1aBcD")
    );
    assert_eq!(
        key("https://www.bilibili.com/video/BV1AbCd"),
        key("https://www.bilibili.com/video/BV1AbCd?utm_source=x")
    );
}
#[test]
fn file_identity_includes_tail() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a");
    let mut bytes = vec![0u8; 4 * 1024 * 1024 + 10];
    std::fs::write(&a, &bytes).unwrap();
    let first = media_key(&MediaSource::LocalFile { path: a.clone() }, Some(1.0)).unwrap();
    *bytes.last_mut().unwrap() = 1;
    std::fs::write(&a, &bytes).unwrap();
    assert_ne!(
        first,
        media_key(&MediaSource::LocalFile { path: a }, Some(1.0)).unwrap()
    );
}
#[test]
fn model_replacement_and_options_invalidate_fingerprint() {
    let t = tempfile::tempdir().unwrap();
    let root = DataRoot {
        path: t.path().into(),
    };
    let model = t.path().join("model.bin");
    std::fs::write(&model, b"aaaa").unwrap();
    let mut cfg = AppConfig::default();
    cfg.tools.whisper_model_path = model.display().to_string();
    let fp = |cfg: &AppConfig| avr_core::asr::engine::processing_fingerprint(&root, cfg).unwrap();
    let a = fp(&cfg);
    std::fs::write(&model, b"bbbb").unwrap();
    let b = fp(&cfg);
    assert_ne!(a, b);
    cfg.asr.prompt = "技术术语".into();
    assert_ne!(b, fp(&cfg));
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
fn compact_marks_actual_unicode_truncation() {
    let text = "中文".repeat(2000);
    let r = avr_core::models::AnalyzeResult {
        video: avr_core::models::VideoInfo {
            id: "x".into(),
            title: "x".into(),
            url: "x".into(),
            duration: 1.0,
            platform: avr_core::models::Platform::Local,
            webpage_url: "x".into(),
        },
        transcript: Some(avr_core::models::Transcript {
            language: "zh".into(),
            source: avr_core::models::TranscriptSource::Asr,
            segments: vec![avr_core::models::TranscriptSegment {
                start: 0.0,
                end: 1.0,
                text,
            }],
        }),
        transcript_path: "text".into(),
        transcript_uri: "file:///text".into(),
        cache_dir: "cache".into(),
        media_key: "key".into(),
        truncated: false,
    };
    let v = avr_core::pipeline::compact_mcp_payload(&r, 3000);
    assert_eq!(v["truncated"], true);
    assert_eq!(v["text"].as_str().unwrap().chars().count(), 3003);
}
#[test]
fn unix_file_uri_is_encoded() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("中文 a.txt");
    std::fs::write(&p, b"x").unwrap();
    let uri = avr_core::paths::to_file_uri(&p);
    assert_eq!(url::Url::parse(&uri).unwrap().to_file_path().unwrap(), p);
    assert!(!uri.starts_with("file:////"));
}

#[tokio::test]
async fn failed_lock_does_not_poison_singleflight() {
    let t = tempfile::tempdir().unwrap();
    let lock = std::fs::File::create(t.path().join(".lock")).unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let mgr = avr_core::jobs::JobManager::new();
    assert!(mgr
        .run_once("x", t.path(), || async { Ok(()) })
        .await
        .is_err());
    let retry = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        mgr.run_once("x", t.path(), || async { Ok(()) }),
    )
    .await;
    assert!(retry.unwrap().is_err());
    drop(lock);
    assert!(mgr
        .run_once("x", t.path(), || async { Ok(()) })
        .await
        .is_ok());
}
#[tokio::test]
async fn cancelled_job_releases_waiters_and_lock() {
    let t = tempfile::tempdir().unwrap();
    let mgr = std::sync::Arc::new(avr_core::jobs::JobManager::new());
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let task = {
        let mgr = mgr.clone();
        let dir = t.path().to_path_buf();
        let started = started.clone();
        tokio::spawn(async move {
            mgr.run_once("x", &dir, || async move {
                started.notify_one();
                std::future::pending::<()>().await;
                Ok(())
            })
            .await
        })
    };
    started.notified().await;
    let mut waiting = Box::pin(mgr.run_once("x", t.path(), || async {
        panic!("等待者不应执行第二份工作")
    }));
    tokio::select! {
        biased;
        _ = &mut waiting => panic!("执行者未结束，等待者不应提前完成"),
        _ = tokio::task::yield_now() => {}
    }
    task.abort();
    let _ = task.await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), &mut waiting)
            .await
            .unwrap()
            .is_err()
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        mgr.run_once("x", t.path(), || async { Ok(()) }),
    )
    .await;
    assert!(result.unwrap().is_ok());
}
#[cfg(unix)]
#[tokio::test]
async fn stderr_pressure_is_drained_and_timeout_reaped() {
    use avr_core::process::{run, ProcessRequest};
    use std::time::Duration;
    let request = |script: &str| ProcessRequest {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: None,
        timeout: Some(Duration::from_secs(3)),
        expected_outputs: vec![],
    };
    let output = run(&request("head -c 2097152 /dev/zero >&2; printf done"))
        .await
        .unwrap();
    assert_eq!(output.stdout, "done");
    assert_eq!(output.stderr.len(), 1024 * 1024);
    let t = tempfile::tempdir().unwrap();
    let marker = t.path().join("pid");
    let mut r = request("echo $$ > pid; exec sleep 30");
    r.cwd = Some(t.path().into());
    r.timeout = Some(Duration::from_millis(100));
    assert!(run(&r).await.is_err());
    let pid = std::fs::read_to_string(marker)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}
#[cfg(unix)]
#[tokio::test]
async fn real_no_audio_video_exports_portable_packet() {
    use avr_core::process::{run, ProcessRequest};
    if !std::path::Path::new("/usr/bin/ffmpeg").is_file() {
        eprintln!("SKIP: ffmpeg unavailable");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let video = t.path().join("中文 视频.mp4");
    let r = ProcessRequest {
        program: "/usr/bin/ffmpeg".into(),
        args: vec![
            "-v".into(),
            "error".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "color=c=blue:s=320x240:r=5:d=1".into(),
            "-c:v".into(),
            "mpeg4".into(),
            video.display().to_string(),
        ],
        cwd: None,
        timeout: Some(std::time::Duration::from_secs(10)),
        expected_outputs: vec![video.clone()],
    };
    run(&r).await.unwrap();
    let mut ctx = avr_core::pipeline::PipelineContext::load(DataRoot {
        path: t.path().join("data"),
    })
    .unwrap();
    ctx.config.tools.ffmpeg_path = "/usr/bin/ffmpeg".into();
    ctx.config.tools.ffprobe_path = "/usr/bin/ffprobe".into();
    let output = t.path().join("packet");
    let opts = avr_core::packet::PacketOptions {
        max_frames: 2,
        ..Default::default()
    };
    let result = avr_core::packet::build_packet(&ctx, &video, &output, &opts)
        .await
        .unwrap();
    assert_eq!(result["frame_count"], 2);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["has_audio"], false);
    assert_eq!(manifest["frames"][0]["height"], 240);
    assert_eq!(manifest["frames"][0]["width"], 320);
    assert!(std::fs::read_to_string(output.join("index.html"))
        .unwrap()
        .contains("data:image/jpeg;base64,"));
    assert!(avr_core::packet::build_packet(&ctx, &video, &output, &opts)
        .await
        .is_err());
}

#[test]
fn partial_cache_reconstructs_text_and_srt() {
    use avr_core::{
        cache::{load_cached_transcript, save_cache},
        models::*,
    };
    let dir = tempfile::tempdir().unwrap();
    let video = VideoInfo {
        id: "local".into(),
        title: "synthetic".into(),
        url: "synthetic.mp4".into(),
        duration: 1.0,
        platform: Platform::Local,
        webpage_url: "synthetic.mp4".into(),
    };
    let transcript = Transcript {
        language: "zh".into(),
        source: TranscriptSource::Asr,
        segments: vec![TranscriptSegment {
            start: 0.0,
            end: 1.0,
            text: "真实重复".into(),
        }],
    };
    let meta = CacheMetadata {
        app_version: "2".into(),
        asr_provider: Some("synthetic".into()),
        asr_model: "synthetic".into(),
        video_id: "local".into(),
        platform: "local".into(),
        source: "asr".into(),
        created_at: "synthetic".into(),
    };
    save_cache(dir.path(), &video, &transcript, &meta).unwrap();
    std::fs::remove_file(dir.path().join("transcript.txt")).unwrap();
    std::fs::remove_file(dir.path().join("subtitle.srt")).unwrap();
    assert!(load_cached_transcript(dir.path()).unwrap().is_some());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("transcript.txt")).unwrap(),
        transcript.full_text()
    );
    assert_eq!(
        avr_core::timeline::parse_srt(
            &std::fs::read_to_string(dir.path().join("subtitle.srt")).unwrap(),
            "zh"
        )
        .segments,
        transcript.segments
    );
}
#[test]
fn low_frame_rate_hints_are_decodable() {
    let probe = avr_core::ffmpeg::MediaProbe {
        duration: 1.0,
        width: 320,
        height: 240,
        frame_rate: 5.0,
        has_audio: false,
    };
    let mut times = sampling_times(1.0, 2.0, 50, &[], &[]).unwrap();
    avr_core::frames::clamp_automatic_times(&mut times, &[], &probe);
    assert_eq!(times, vec![0.0, 0.8]);
}

#[test]
fn media_keys_reject_cross_platform_path_components() {
    for value in ["", ".", "..", "C:escape", "a/b", "a\\b"] {
        assert!(!avr_core::media::valid_media_key(value));
    }
    let key = media_key(
        &MediaSource::Url {
            url: "https://www.bilibili.com/video/BV1ABC?p=..%2Fescape".into(),
        },
        None,
    )
    .unwrap();
    assert!(avr_core::media::valid_media_key(&key));
}
