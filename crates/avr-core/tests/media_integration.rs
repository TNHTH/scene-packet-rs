use avr_core::{
    ffmpeg::FfmpegContext,
    keyframes::{dhash, sample_video, SAMPLE_H, SAMPLE_W},
};
use std::{path::Path, process::Command};

fn ffmpeg() -> Option<FfmpegContext> {
    let present = Command::new("ffmpeg").arg("-version").output().is_ok();
    if !present {
        assert!(
            std::env::var_os("AVR_REQUIRE_FFMPEG").is_none(),
            "需要 ffmpeg"
        );
        eprintln!("SKIP: ffmpeg unavailable");
        return None;
    }
    Some(FfmpegContext {
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
    })
}
fn make_video(path: &Path, filter: &str, duration: &str, rate: &str) {
    let out = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=160x90:rate={rate}:duration={duration}"),
            "-vf",
            filter,
            "-vsync",
            "0",
            "-c:v",
            "ffv1",
            "-enc_time_base",
            "1/1000",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn vfr_samples_retain_actual_pts_and_pixels() {
    let Some(ctx) = ffmpeg() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let video = d.path().join("vfr.mkv");
    make_video(
        &video,
        "settb=1/1000,setpts='if(eq(N,0),0,if(eq(N,1),1500,2500))'",
        "3",
        "1",
    );
    let samples = sample_video(&ctx, &video, 1.0, d.path()).await.unwrap();
    assert_eq!(
        samples.iter().map(|s| s.time).collect::<Vec<_>>(),
        vec![0.0, 1.5, 2.5]
    );
    for sample in samples {
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-ss", &sample.time.to_string(), "-i"])
            .arg(&video)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale=160:90,format=gray",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), SAMPLE_W * SAMPLE_H);
        assert_eq!(sample.hash, dhash(&out.stdout, SAMPLE_W, SAMPLE_H));
    }
}

#[tokio::test]
async fn concurrent_short_sampling_keeps_first_frame_and_cleans_temporary_files() {
    let Some(ctx) = ffmpeg() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let video = d.path().join("short.mkv");
    make_video(&video, "null", "0.1", "30");
    let (a, b) = tokio::join!(
        sample_video(&ctx, &video, 2.0, d.path()),
        sample_video(&ctx, &video, 2.0, d.path())
    );
    for result in [a, b] {
        let samples = result.unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].time, 0.0);
    }
    assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    let frames = avr_core::frames::extract_frames(
        &ctx,
        &video,
        &[0.0],
        &d.path().join("frames"),
        1280,
        avr_core::frames::FrameDelivery::Both,
    )
    .await
    .unwrap();
    assert_eq!((frames[0].width, frames[0].height), (160, 90));
    assert!(avr_core::frames::extract_frames(
        &ctx,
        &video,
        &[20.0],
        &d.path().join("frames"),
        1280,
        avr_core::frames::FrameDelivery::Path
    )
    .await
    .is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn subtitle_download_ignores_stale_languages() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("subtitle.en.vtt"),
        "WEBVTT\n\n00:00.000 --> 00:01.000\nOLD ENGLISH\n",
    )
    .unwrap();
    let tool = d.path().join("ytdlp");
    std::fs::write(&tool, "#!/bin/sh\nwhile [ $# -gt 0 ]; do\nif [ \"$1\" = '-o' ]; then shift; out=$1; fi\nshift\ndone\nprintf '1\\n00:00:00,000 --> 00:00:01,000\\nNEW CHINESE\\n' > \"$out.zh.srt\"\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut cfg = avr_core::config::AppConfig::default();
    cfg.tools.ytdlp_path = tool.display().to_string();
    let y = avr_core::extract::ytdlp::YtDlpExtractor::new(
        &avr_core::root::DataRoot {
            path: d.path().join("root"),
        },
        &cfg,
    )
    .unwrap();
    let t = y
        .fetch_subtitles_with_info(
            "https://www.bilibili.com/video/BV1xx",
            d.path(),
            &serde_json::json!({"subtitles":{"zh":[{"ext":"srt"}]}}),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(t.full_text(), "NEW CHINESE");
}

#[tokio::test]
async fn keyframe_cache_invalidates_when_transcript_changes() {
    let Some(ctx) = ffmpeg() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let video = d.path().join("video.mkv");
    make_video(&video, "null", "1", "5");
    let cfg = avr_core::keyframes::KeyframeConfig::default();
    avr_core::keyframes::load_or_compute(&ctx, &video, d.path(), &cfg, None)
        .await
        .unwrap();
    std::fs::remove_file(&video).unwrap();
    assert!(
        avr_core::keyframes::load_or_compute(&ctx, &video, d.path(), &cfg, None)
            .await
            .is_ok()
    );
    let transcript =
        avr_core::timeline::parse_srt("1\n00:00:00,000 --> 00:00:01,000\n新转写\n", "zh");
    assert!(
        avr_core::keyframes::load_or_compute(&ctx, &video, d.path(), &cfg, Some(&transcript))
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn visual_download_uses_full_timeline_and_commits_only_valid_video() {
    use std::os::unix::fs::PermissionsExt;
    let Some(ctx) = ffmpeg() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let video = d.path().join("fixture.mkv");
    make_video(&video, "null", "1", "5");
    let tool = d.path().join("ytdlp");
    let script=format!("#!/bin/sh\nwhile [ $# -gt 0 ]; do\nif [ \"$1\" = '--download-sections' ]; then exit 9; fi\nif [ \"$1\" = '-o' ]; then shift; out=$1; fi\nshift\ndone\ncp '{}' \"$out\"\n",video.display());
    std::fs::write(&tool, script).unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut cfg = avr_core::config::AppConfig::default();
    cfg.tools.ytdlp_path = tool.display().to_string();
    let y = avr_core::extract::ytdlp::YtDlpExtractor::new(
        &avr_core::root::DataRoot {
            path: d.path().join("root"),
        },
        &cfg,
    )
    .unwrap();
    let cache = d.path().join("cache");
    let path = avr_core::frames::resolve_visual_source(
        &y,
        &ctx,
        "https://www.bilibili.com/video/BV1test",
        &cache,
        5000.0,
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(&video).unwrap()
    );
    std::fs::write(&video, b"invalid video").unwrap();
    let bad = d.path().join("bad-cache");
    assert!(avr_core::frames::resolve_visual_source(
        &y,
        &ctx,
        "https://www.bilibili.com/video/BV1test",
        &bad,
        0.0
    )
    .await
    .is_err());
    assert!(!bad.join("visual_full.mp4").exists());
    std::fs::remove_file(&tool).unwrap();
    assert_eq!(
        avr_core::frames::resolve_visual_source(
            &y,
            &ctx,
            "https://www.bilibili.com/video/BV1test",
            &cache,
            5000.0
        )
        .await
        .unwrap(),
        path
    );
}

#[cfg(unix)]
#[tokio::test]
async fn failed_packet_can_retry_without_partial_export() {
    use std::os::unix::fs::PermissionsExt;
    let Some(_) = ffmpeg() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let video = d.path().join("fixture.mkv");
    make_video(&video, "null", "1", "10");
    let tool = d.path().join("ff-wrapper");
    let marker = d.path().join("first-frame");
    let script=format!("#!/bin/sh\nfor arg do\nif [ \"$arg\" = '-frames:v' ]; then\nif [ -f '{}' ]; then exit 1; fi\ntouch '{}'\nfi\ndone\nexec ffmpeg \"$@\"\n",marker.display(),marker.display());
    std::fs::write(&tool, script).unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut cfg = avr_core::config::AppConfig::default();
    cfg.tools.ffmpeg_path = tool.display().to_string();
    cfg.tools.ffprobe_path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("ffprobe"))
        .find(|p| p.is_file())
        .unwrap()
        .display()
        .to_string();
    let ctx = avr_core::pipeline::PipelineContext {
        root: avr_core::root::DataRoot {
            path: d.path().join("root"),
        },
        config: cfg,
    };
    let output = d.path().join("packet");
    let options = avr_core::packet::PacketOptions {
        visual_only: true,
        interval: 0.3,
        max_frames: 10,
        ..Default::default()
    };
    assert!(
        avr_core::packet::build_packet(&ctx, &video, &output, &options)
            .await
            .is_err()
    );
    assert!(marker.exists(), "必须先成功截图一次，再注入失败");
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
    std::fs::write(&tool, "#!/bin/sh\nexec ffmpeg \"$@\"\n").unwrap();
    avr_core::packet::build_packet(&ctx, &video, &output, &options)
        .await
        .unwrap();
    let manifest = std::fs::read(output.join("manifest.json")).unwrap();
    assert!(
        avr_core::packet::build_packet(&ctx, &video, &output, &options)
            .await
            .is_err()
    );
    assert_eq!(
        manifest,
        std::fs::read(output.join("manifest.json")).unwrap()
    );
}
