//! 对本地视频跑选帧并打印结果（效果集用）。可选：导出选中帧 JPEG。
//! 用法: cargo run -p avr-core --example keyframes -- <video> [transcript.json] [out_dir]
//! 不给转写时按“讲话稀少”处理。输出 JSON：参数、采样数、场景切点、选中帧（n, time, start, end, content）。
use avr_core::keyframes::{sample_video, select, KeyframeConfig};
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let video = PathBuf::from(&a[1]);
    let segs: Vec<(f64, f64, usize)> = a
        .get(2)
        .filter(|p| !p.is_empty() && *p != "-")
        .map(|p| {
            let v: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(p).expect("read")).expect("json");
            v["segments"]
                .as_array()
                .expect("segments")
                .iter()
                .map(|s| {
                    let t = s["text"].as_str().unwrap_or("");
                    (
                        s["start"].as_f64().unwrap_or(0.0),
                        s["end"].as_f64().unwrap_or(0.0),
                        avr_core::asr::postprocess::speech_units(t),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let ffmpeg = avr_core::ffmpeg::FfmpegContext {
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
    };
    let cfg = KeyframeConfig::default();
    let tmp = std::env::temp_dir().join("avr-keyframes");
    let samples = sample_video(&ffmpeg, &video, cfg.interval_secs, &tmp)
        .await
        .expect("sample");
    let duration = avr_core::ffmpeg::probe_duration(&ffmpeg, &video)
        .await
        .expect("probe");
    let scenes: Vec<f64> = avr_core::ffmpeg::detect_scenes(&ffmpeg, &video, 0.3)
        .await
        .expect("scenes")
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let units = |x: f64, y: f64| -> usize {
        segs.iter()
            .filter(|s| s.0 < y && s.1 > x)
            .map(|s| s.2)
            .sum()
    };
    let picked = select(&samples, duration, &cfg, &scenes, &units);
    if let Some(out) = a.get(3) {
        std::fs::create_dir_all(out).expect("mkdir");
        for k in &picked {
            let p = PathBuf::from(out).join(format!("{:03}_{:07.1}s.jpg", k.n, k.time));
            avr_core::ffmpeg::capture_frame(&ffmpeg, &video, k.time, &p, 1280)
                .await
                .expect("cap");
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "video": video, "duration": duration, "config": cfg,
            "samples": samples.len(), "scene_cuts": scenes,
            "per_hour": picked.len() as f64 / (duration / 3600.0),
            "frames": picked,
        }))
        .expect("json")
    );
}
