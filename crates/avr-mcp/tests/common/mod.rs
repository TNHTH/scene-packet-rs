//! 集成测试共用：ffmpeg 探测、测试视频生成、固定转写、FakeTools 流水线。
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use avr_core::config::AppConfig;
use avr_core::ffmpeg::FfmpegContext;
use avr_core::jobs::JobManager;
use avr_core::models::{AnalyzeResult, Transcript, TranscriptSegment, TranscriptSource};
use avr_core::pipeline::{analyze, AnalyzeOptions, Deps, PipelineContext, Source};
use avr_core::root::DataRoot;
use avr_core::tools::runtime::fake::FakeTools;

pub const VIDEO_SECS: f64 = 6.0;

pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// 返回 ffmpeg 上下文；找不到时按 AVR_REQUIRE_FFMPEG 决定跳过还是失败。
pub fn ffmpeg_or_skip() -> Option<FfmpegContext> {
    match (find_on_path("ffmpeg"), find_on_path("ffprobe")) {
        (Some(ffmpeg), Some(ffprobe)) => Some(FfmpegContext { ffmpeg, ffprobe }),
        _ => {
            if std::env::var("AVR_REQUIRE_FFMPEG").as_deref() == Ok("1") {
                panic!("AVR_REQUIRE_FFMPEG=1 但 PATH 上找不到 ffmpeg/ffprobe");
            }
            eprintln!("ffmpeg baseline: SKIPPED (no ffmpeg on PATH)");
            None
        }
    }
}

/// testsrc2 2s + 红 2s + 蓝 2s（两处硬切，场景检测稳定），配 440Hz 正弦音。
pub fn make_video(ff: &FfmpegContext, dir: &Path) -> PathBuf {
    let out = dir.join("baseline.mp4");
    let status = Command::new(&ff.ffmpeg)
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x240:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x240:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=6",
            "-filter_complex",
            "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]",
            "-map",
            "[v]",
            "-map",
            "3:a",
            "-pix_fmt",
            "yuv420p",
            "-shortest",
        ])
        .arg(&out)
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "ffmpeg 生成测试视频失败");
    out
}

/// 固定转写：60 段，均匀铺满 6 秒，含“如图”“幻灯片”等视觉线索。
pub fn fixed_transcript() -> Transcript {
    const LINES: [&str; 6] = [
        "今天我们来讲视频上下文包的整体设计思路",
        "如图所示左边是输入右边是输出的结构化结果",
        "首先流水线会检查缓存是否命中以及版本是否一致",
        "接着抽取音频并调用本地语音识别得到逐句转写",
        "这张幻灯片总结了三种入口共用同一个分析接口",
        "最后把转写和关键帧整理成大模型容易读取的格式",
    ];
    let n = 60;
    let step = VIDEO_SECS / n as f64;
    Transcript {
        language: "zh".into(),
        source: TranscriptSource::Asr,
        segments: (0..n)
            .map(|i| TranscriptSegment {
                start: (i as f64 * step * 100.0).round() / 100.0,
                end: ((i + 1) as f64 * step * 100.0).round() / 100.0,
                text: format!("第{}句：{}", i + 1, LINES[i % LINES.len()]),
                suspect: None,
            })
            .collect(),
    }
}

pub fn test_ctx(dir: &Path) -> PipelineContext {
    PipelineContext {
        root: DataRoot {
            path: dir.join("root"),
        },
        config: AppConfig::default(),
    }
}

pub fn fake_deps() -> Deps {
    let fake = Arc::new(FakeTools {
        transcript: Some(fixed_transcript()),
        ..Default::default()
    });
    Deps::new(Arc::new(JobManager::new())).with_tools(fake)
}

pub async fn run_pipeline(dir: &Path, video: &Path) -> AnalyzeResult {
    let ctx = test_ctx(dir);
    let deps = fake_deps();
    analyze(
        &ctx,
        &Source::Local(video.to_path_buf()),
        &AnalyzeOptions::new(),
        &deps,
    )
    .await
    .expect("analyze")
}
