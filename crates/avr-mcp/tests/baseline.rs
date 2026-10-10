//! P3-1 输出质量基线：ffmpeg lavfi 现生成测试视频，转写用 FakeTools 固定结果，
//! 时间轴（scene 检测）与关键帧走真实 ffmpeg；对结构做 insta 快照。
//!
//! - 找不到 ffmpeg：跳过；但 `AVR_REQUIRE_FFMPEG=1` 时直接失败（CI 设置）。
//! - token 统计（tiktoken o200k_base）在 `#[ignore]` 测试 `token_stats` 里，
//!   普通 `cargo test` 不跑：`cargo test -p avr-mcp --test baseline -- --ignored --nocapture`。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use avr_core::config::AppConfig;
use avr_core::ffmpeg::FfmpegContext;
use avr_core::frames::{extract_frames, suggest_visual_hints, FrameDelivery};
use avr_core::jobs::JobManager;
use avr_core::models::{AnalyzeResult, Transcript, TranscriptSegment, TranscriptSource};
use avr_core::pipeline::{analyze, AnalyzeOptions, Deps, PipelineContext, Source};
use avr_core::root::DataRoot;
use avr_core::timeline::segments_to_srt;
use avr_core::tools::runtime::fake::FakeTools;
use avr_mcp::payload::compact_mcp_payload;
use serde_json::json;

const VIDEO_SECS: f64 = 6.0;

fn find_on_path(name: &str) -> Option<PathBuf> {
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
fn ffmpeg_or_skip() -> Option<FfmpegContext> {
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
fn make_video(ff: &FfmpegContext, dir: &Path) -> PathBuf {
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
            })
            .collect(),
    }
}

pub async fn run_pipeline(dir: &Path, video: &Path) -> AnalyzeResult {
    let ctx = PipelineContext {
        root: DataRoot {
            path: dir.join("root"),
        },
        config: AppConfig::default(),
    };
    let fake = Arc::new(FakeTools {
        transcript: Some(fixed_transcript()),
        ..Default::default()
    });
    let deps = Deps::new(Arc::new(JobManager::new())).with_tools(fake);
    analyze(
        &ctx,
        &Source::Local(video.to_path_buf()),
        &AnalyzeOptions::new(),
        &deps,
    )
    .await
    .expect("analyze")
}

#[tokio::test]
async fn output_quality_baseline() {
    let Some(ff) = ffmpeg_or_skip() else { return };
    let tmp = tempfile::TempDir::new().unwrap();
    let video = make_video(&ff, tmp.path());
    let result = run_pipeline(tmp.path(), &video).await;
    let transcript = result.transcript.clone().unwrap();

    // 时间轴：真实 ffmpeg 场景检测 + 文本线索。
    let scenes = avr_core::ffmpeg::detect_scenes(&ff, &video, 0.3)
        .await
        .unwrap();
    let hints = suggest_visual_hints(&transcript, &scenes, 8);
    let times: Vec<f64> = hints
        .iter()
        .map(|h| h.time)
        .filter(|t| *t < VIDEO_SECS - 0.2)
        .collect();
    let frames = extract_frames(
        &ff,
        &video,
        &times,
        &tmp.path().join("frames"),
        320,
        FrameDelivery::Path,
    )
    .await
    .unwrap();

    // 关键帧只记结构：帧数范围、时间戳是否落在某个分段内、文件是否生成。
    let in_segment = |t: f64| {
        transcript
            .segments
            .iter()
            .any(|s| s.start <= t && t <= s.end)
    };
    let keyframes = json!({
        "count_in_range_1_to_8": (1..=8).contains(&frames.len()),
        "scene_cuts_detected_at_least_2": scenes.len() >= 2,
        "all_times_within_a_segment": frames.iter().all(|f| in_segment(f.time)),
        "all_files_exist": frames.iter().all(|f| Path::new(&f.path).is_file()),
        "fields": frames.first().map(|f| {
            let mut keys: Vec<String> = serde_json::to_value(f).unwrap()
                .as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        }),
    });
    let timeline = json!({
        "srt_cue_count": segments_to_srt(&transcript).matches(" --> ").count(),
        "srt_head": segments_to_srt(&transcript).lines().take(8).collect::<Vec<_>>(),
        "text_cue_hints": hints.iter().filter(|h| h.segment_id.is_some()).count(),
    });

    insta::assert_json_snapshot!("transcript", result, {
        ".transcript_path" => "[path]",
        ".transcript_uri" => "[uri]",
        ".cache_dir" => "[path]",
        ".media_key" => "[media_key]",
        ".video.id" => "[media_key]",
        ".video.url" => "[path]",
        ".video.webpage_url" => "[path]",
    });
    insta::assert_json_snapshot!("timeline", timeline);
    insta::assert_json_snapshot!("keyframes", keyframes);

    let compact = serde_json::to_string(&compact_mcp_payload(&result, 3000)).unwrap();
    println!(
        "ffmpeg baseline: RAN (compact payload {} bytes)",
        compact.len()
    );
}

/// P3-2 往返：摘要索引 + 分页取回的转写 + 按编号取回的帧，拼起来与 full payload 逐项一致。
#[tokio::test]
async fn summary_index_roundtrip_reconstructs_full_payload() {
    use avr_core::models::format_inline_text;
    use avr_mcp::index::{frame_ref, read_transcript, summary_index};

    let tmp = tempfile::TempDir::new().unwrap();
    let media = tmp.path().join("clip.mp4");
    std::fs::write(&media, b"fake").unwrap();
    let result = run_pipeline(tmp.path(), &media).await;
    let full = avr_mcp::payload::full_mcp_payload(&result);
    let index = summary_index(&result);

    // 1. 分页取回全部分段（小页，强制多次翻页）。
    let mut segments = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let page = read_transcript(&result, 0.0, f64::MAX, cursor.as_deref(), 200).unwrap();
        pages += 1;
        segments.extend(page.segments);
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert!(pages > 1, "应发生分页");
    assert_eq!(
        segments.len(),
        index["segment_count"].as_u64().unwrap() as usize
    );
    assert!(segments.windows(2).all(|w| w[1].index == w[0].index + 1));

    // 2. 按编号取回每个帧：编号与索引一致，时间落在对应 section 内。
    let sections = index["sections"].as_array().unwrap();
    for (n, sec) in sections.iter().enumerate() {
        let f = frame_ref(&result, n).unwrap();
        assert_eq!(serde_json::to_value(&f).unwrap(), index["frames"][n]);
        let (s, e) = (sec["start"].as_f64().unwrap(), sec["end"].as_f64().unwrap());
        assert!(s <= f.time && f.time <= e);
    }
    assert!(frame_ref(&result, sections.len()).is_err());

    // 3. 拼回 full payload，逐项比较。
    let transcript = avr_core::models::Transcript {
        language: index["language"].as_str().unwrap().into(),
        source: serde_json::from_value(index["source"].clone()).unwrap(),
        segments: segments
            .iter()
            .map(|s| avr_core::models::TranscriptSegment {
                start: s.start,
                end: s.end,
                text: s.text.clone(),
            })
            .collect(),
    };
    let mut rebuilt = serde_json::Map::new();
    for key in [
        "title",
        "platform",
        "duration",
        "video_id",
        "source",
        "language",
        "truncated",
        "transcript_path",
        "transcript_uri",
        "text_file",
        "cache_dir",
        "media_key",
    ] {
        rebuilt.insert(key.into(), index[key].clone());
    }
    rebuilt.insert(
        "segments".into(),
        serde_json::to_value(&transcript.segments).unwrap(),
    );
    rebuilt.insert(
        "text".into(),
        json!(format_inline_text(
            &transcript,
            index["truncated"].as_bool().unwrap()
        )),
    );
    let full = full.as_object().unwrap();
    assert_eq!(rebuilt.len(), full.len(), "字段集合一致");
    for (k, v) in full {
        assert_eq!(rebuilt.get(k), Some(v), "字段 {k} 不一致");
    }
}

/// token 统计（o200k_base 参考值）。`--ignored` 时才跑，不需要 ffmpeg。
#[test]
#[ignore = "token 统计：cargo test -p avr-mcp --test baseline -- --ignored --nocapture"]
fn token_stats() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let media = tmp.path().join("clip.mp4");
    std::fs::write(&media, b"fake").unwrap();
    let result = rt.block_on(run_pipeline(tmp.path(), &media));
    let bpe = tiktoken_rs::o200k_base().unwrap();
    for (name, v) in [
        ("index", avr_mcp::index::summary_index(&result)),
        ("compact", compact_mcp_payload(&result, 3000)),
        ("full", avr_mcp::payload::full_mcp_payload(&result)),
    ] {
        let s = serde_json::to_string(&v).unwrap();
        let tokens = bpe.encode_with_special_tokens(&s).len();
        println!(
            "token-stats {name}: bytes={} tokens_o200k={tokens}",
            s.len()
        );
    }
}
