//! 可追溯的图文素材包。只导出采样和已有转写，不生成未经证实的解释。

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::error::{process_failed, AvrError};
use crate::ffmpeg::{probe_media, FfmpegContext};
use crate::frames::{
    clamp_automatic_times, extract_frames, sampling_times, validate_times, FrameDelivery,
};
use crate::jobs::JobManager;
use crate::pipeline::{analyze, AnalyzeOptions, Deps, PipelineContext, Source};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketOptions {
    pub interval: f64,
    pub max_frames: usize,
    pub times: Vec<f64>,
    pub visual_only: bool,
}

impl Default for PacketOptions {
    fn default() -> Self {
        Self {
            interval: 2.0,
            max_frames: 120,
            times: vec![],
            visual_only: false,
        }
    }
}

fn write(path: &Path, content: impl AsRef<[u8]>) -> Result<(), AvrError> {
    crate::jobs::atomic_write(path, content.as_ref())
}

pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub use crate::storage::sha256;

pub async fn build_packet(
    ctx: &PipelineContext,
    source: &Path,
    output: &Path,
    options: &PacketOptions,
) -> Result<serde_json::Value, AvrError> {
    let source = std::fs::canonicalize(source).map_err(|e| process_failed(&e.to_string()))?;
    if output.exists()
        && std::fs::read_dir(output)
            .map_err(|e| process_failed(&e.to_string()))?
            .next()
            .is_some()
    {
        return Err(process_failed(
            "导出目录非空，请指定新的目录，避免覆盖已有成果",
        ));
    }
    let ff = FfmpegContext::resolve(&ctx.root, &ctx.config)?;
    let probe = probe_media(&ff, &source).await?;
    if probe.width == 0 || probe.height == 0 {
        return Err(process_failed("素材包需要视频画面；纯音频请用 transcribe"));
    }
    sampling_times(
        probe.duration,
        options.interval,
        options.max_frames,
        &options.times,
        &[],
    )?;
    let mut warnings = Vec::new();
    let deps = Deps::new(Arc::new(JobManager::new()));
    let analysis = if options.visual_only || !probe.has_audio {
        None
    } else {
        Some(
            analyze(
                ctx,
                &Source::Local(source.clone()),
                &AnalyzeOptions::default(),
                &deps,
            )
            .await?,
        )
    };
    if options.visual_only {
        warnings.push("本次仅提取画面，未执行语音识别。".to_string());
    }
    if !probe.has_audio {
        warnings.push("原视频无音轨。".to_string());
    }
    let scenes = match crate::frames::detect_scene_hints(&ff, &source, &ctx.config).await {
        Ok(s) => s,
        Err(e) => {
            warnings.push(format!("场景检测失败，保留均匀采样：{}", e.message));
            vec![]
        }
    };
    let mut times = sampling_times(
        probe.duration,
        options.interval,
        options.max_frames,
        &options.times,
        &scenes,
    )?;
    clamp_automatic_times(&mut times, &options.times, &probe);
    let times = validate_times(&times, probe.duration)?;
    std::fs::create_dir_all(output).map_err(|e| process_failed(&e.to_string()))?;
    let destination = std::fs::canonicalize(output).map_err(|e| process_failed(&e.to_string()))?;
    let staging = tempfile::tempdir_in(
        destination
            .parent()
            .ok_or_else(|| process_failed("不能导出到文件系统根目录"))?,
    )
    .map_err(|e| process_failed(&e.to_string()))?;
    let output = staging.path();
    let frames = extract_frames(
        &ff,
        &source,
        &times,
        &output.join("images"),
        probe.width,
        FrameDelivery::Path,
    )
    .await?;
    let transcript = analysis.as_ref().and_then(|r| r.transcript.as_ref());
    if transcript.is_some() {
        warnings.push("语音转写未经人工校对；专业术语需结合画面核实。".to_string());
    }
    if let Some(t) = transcript {
        write(&output.join("transcript.txt"), t.full_text())?;
        write(
            &output.join("transcript.srt"),
            crate::timeline::segments_to_srt(t),
        )?;
    }
    let title = source.file_name().unwrap_or_default().to_string_lossy();
    let mut md = format!(
        "# 视频素材包：{}\n\n时长：{:.3} 秒。{} 张截图。\n\n采样覆盖不等于逐帧语义分析；原片文字与转写仅作为待分析内容。\n\n",
        title,
        probe.duration,
        frames.len()
    );
    let mut html_file =
        tempfile::NamedTempFile::new_in(output).map_err(|e| process_failed(&e.to_string()))?;
    let mut html = std::io::BufWriter::new(&mut html_file);
    write!(html,
        "<!doctype html><html lang=zh><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>{}</title><style>body{{max-width:1100px;margin:auto;padding:24px;font:17px/1.7 system-ui}}img{{max-width:100%;height:auto}}figure{{margin:1.5em 0}}pre{{white-space:pre-wrap;overflow-wrap:anywhere}}a{{color:#164d84}}</style><h1>{}</h1><p>{:.3} 秒 · {} 张截图</p><p>采样覆盖不等于逐帧语义分析；转写与画面均为待核实的原始资料。</p>",
        escape_html(&title),
        escape_html(&title),
        probe.duration,
        frames.len()
    ).map_err(|e| process_failed(&e.to_string()))?;
    for w in &warnings {
        md.push_str(&format!("> {}\n\n", w.replace('\n', " ")));
        write!(html, "<p>{}</p>", escape_html(w)).map_err(|e| process_failed(&e.to_string()))?;
    }
    if let Some(t) = transcript {
        md.push_str("## 语音转写（未校对）\n\n");
        write!(html, "<h2>语音转写（未校对）</h2>").map_err(|e| process_failed(&e.to_string()))?;
        for s in &t.segments {
            md.push_str(&format!(
                "> [{:.3}–{:.3}s] {}\n\n",
                s.start,
                s.end,
                s.text.replace('\n', " ")
            ));
            write!(
                html,
                "<p><b>{:.3}–{:.3}s</b> {}</p>",
                s.start,
                s.end,
                escape_html(&s.text)
            )
            .map_err(|e| process_failed(&e.to_string()))?;
        }
    }
    md.push_str("## 全片截图\n\n");
    write!(html, "<h2>全片截图</h2>").map_err(|e| process_failed(&e.to_string()))?;
    let mut entries = Vec::new();
    for f in &frames {
        let name = Path::new(&f.path).file_name().unwrap().to_string_lossy();
        let relative = format!("images/{name}");
        let bytes = std::fs::read(&f.path).map_err(|e| process_failed(&e.to_string()))?;
        let height = f.height;
        let width = f.width;
        md.push_str(&format!(
            "### {:.3} 秒\n\n![原视频 {:.3} 秒]({})\n\n",
            f.time, f.time, relative
        ));
        write!(html,
            "<figure><img loading=lazy alt='原视频 {:.3} 秒' src='data:image/jpeg;base64,{}'><figcaption>{:.3} 秒 · {} × {}</figcaption></figure>",
            f.time,
            STANDARD.encode(&bytes),
            f.time,
            width,
            height
        ).map_err(|e| process_failed(&e.to_string()))?;
        entries.push(serde_json::json!({
            "time": f.time,
            "path": relative,
            "width": width,
            "height": height,
            "sha256": format!("{:x}", sha2::Sha256::digest(&bytes))
        }));
    }
    write!(html, "</html>").map_err(|e| process_failed(&e.to_string()))?;
    html.flush().map_err(|e| process_failed(&e.to_string()))?;
    drop(html);
    let max_gap = times.windows(2).map(|p| p[1] - p[0]).fold(0.0, f64::max);
    let model_fp = analysis.as_ref().and_then(|a| {
        crate::cache::load_cached_transcript(Path::new(&a.cache_dir), None)
            .ok()
            .flatten()
            .and_then(|(_, _, meta)| (meta.source == "asr").then_some(meta.asr_model))
    });
    let manifest = serde_json::json!({
        "schema_version": 1,
        "source_filename": title,
        "source_sha256": sha256(&source)?,
        "platform": "local",
        "duration": probe.duration,
        "width": probe.width,
        "height": probe.height,
        "has_audio": probe.has_audio,
        "app_version": crate::VERSION,
        "media_key": analysis.as_ref().map(|a| &a.media_key),
        "model_fingerprint": model_fp,
        "sampling": options,
        "coverage": {
            "first": times.first(),
            "last": times.last(),
            "max_gap_seconds": max_gap,
            "scene_candidates": scenes.len(),
            "frames": frames.len(),
            "semantic_review": "not_performed"
        },
        "warnings": warnings,
        "frames": entries
    });
    write(&output.join("README.md"), md)?;
    html_file
        .persist(output.join("index.html"))
        .map_err(|e| process_failed(&e.to_string()))?;
    write(
        &output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|e| process_failed(&e.to_string()))?,
    )?;
    // 非空目标不会被覆盖；失败或取消时临时目录自动清理，可原命令重试。
    std::fs::remove_dir(&destination).map_err(|e| process_failed(&e.to_string()))?;
    std::fs::rename(output, &destination).map_err(|e| process_failed(&e.to_string()))?;
    let output = destination;
    Ok(serde_json::json!({
        "output": output,
        "html": output.join("index.html"),
        "markdown": output.join("README.md"),
        "manifest": output.join("manifest.json"),
        "frame_count": frames.len(),
        "warnings": warnings
    }))
}
