//! 板书 / 幻灯片选帧（P5-1，ADR 0011）。
//!
//! 流程：
//! 1. 每 `interval_secs` 秒抽一张 [`SAMPLE_W`]×[`SAMPLE_H`] 灰度小图（ffmpeg rawvideo，一次调用）。
//! 2. 每张算 64 位差值哈希（dHash）和“有效内容”分数（边缘像素占比：板书 / 文字 / 图表边缘多，
//!    人挡住时边缘少）。
//! 3. 切区间：与当前区间锚点帧的哈希距离 ≥ `cut_hamming`，且后面 `confirm_samples` 张也都偏离锚点
//!    （抗镜头晃动 / 一闪而过的遮挡），才在此处切开。逐笔书写、PPT 逐条出现的动画只让画面缓慢变化，
//!    累积到阈值前都留在同一区间。
//! 4. 抗碎切：短于 `min_interval_secs` 的区间并入前一个；相邻区间代表帧哈希距离 ≤ `merge_hamming` 也合并。
//! 5. 场景检测（原 `gt(scene,…)`）的时间点作为额外切点，距已有切点 `scene_dedupe_secs` 内的丢弃。
//! 6. 讲话稀少（每分钟少于 `sparse_units_per_min` 字/词）的区间若长于 `sparse_split_secs`，再按该长度细分。
//! 7. 每个区间取有效内容分数最高的一帧（平分取较晚的，即擦掉 / 翻页前更完整的一帧）。
//! 8. 去重：与已选帧哈希距离 ≤ `merge_hamming` 的不再选（翻回同一页）。
//! 9. 上限：`max(min_frames, ceil(时长小时 × max_per_hour))`，超出按有效内容分数取前若干，再按时间排序。
//!    `min_frames` 只是下限预算，不会为凑数选重复画面。

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AvrError;
use crate::ffmpeg::FfmpegContext;
use crate::process::{run, ProcessRequest};

pub const SAMPLE_W: usize = 160;
pub const SAMPLE_H: usize = 90;

/// 选帧参数（`[keyframes]` 配置节）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KeyframeConfig {
    pub interval_secs: f64,
    pub cut_hamming: u32,
    pub confirm_samples: usize,
    pub min_interval_secs: f64,
    pub merge_hamming: u32,
    pub scene_dedupe_secs: f64,
    pub sparse_units_per_min: f64,
    pub sparse_split_secs: f64,
    pub max_per_hour: f64,
    pub min_frames: usize,
    /// 前一帧边缘被后一帧（3×3 邻域）覆盖的比例 ≥ 此值，视为“逐步出现”（PPT 动画 / 板书续写），
    /// 只保留后一帧。
    pub contain_ratio: f32,
    /// 返回给 AI 的图片长边（像素）。
    pub image_long_side: u32,
    /// 返回给 AI 的 JPEG 质量（1–100）。
    pub jpeg_quality: u8,
    /// `get_frame` 单次最多返回的图片数。
    pub max_frames_per_call: usize,
    /// 转写稀少（< `sparse_units_per_min`）时 analyze 首次返回附带的板书图数量上限。
    pub sparse_attach: usize,
}

impl Default for KeyframeConfig {
    fn default() -> Self {
        Self {
            interval_secs: 2.0,
            cut_hamming: 14,
            confirm_samples: 2,
            min_interval_secs: 6.0,
            merge_hamming: 6,
            scene_dedupe_secs: 4.0,
            sparse_units_per_min: 60.0,
            sparse_split_secs: 60.0,
            max_per_hour: 40.0,
            min_frames: 12,
            contain_ratio: 0.85,
            image_long_side: 1280,
            jpeg_quality: 85,
            max_frames_per_call: 4,
            sparse_attach: 4,
        }
    }
}

/// 一张采样帧。
#[derive(Debug, Clone)]
pub struct Sample {
    pub time: f64,
    pub hash: u64,
    pub content: f32,
    /// 边缘位图（SAMPLE_W×SAMPLE_H 位，行优先）；空 = 未知（测试用），不参与“包含”判断。
    pub edges: Vec<u64>,
}

/// 选中的一帧及其代表的区间。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Keyframe {
    /// 帧编号（按时间排序后的序号，从 0 开始）。
    pub n: usize,
    pub time: f64,
    pub start: f64,
    pub end: f64,
    pub content: f32,
    #[serde(skip)]
    pub hash: u64,
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 灰度图（w×h）的 dHash：缩到 9×8 取均值，比较左右相邻。
pub fn dhash(px: &[u8], w: usize, h: usize) -> u64 {
    let mut cells = [0f32; 72];
    for (cy, row) in cells.chunks_mut(9).enumerate() {
        for (cx, cell) in row.iter_mut().enumerate() {
            let (x0, x1) = (cx * w / 9, ((cx + 1) * w / 9).max(cx * w / 9 + 1));
            let (y0, y1) = (cy * h / 8, ((cy + 1) * h / 8).max(cy * h / 8 + 1));
            let mut sum = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += px[y * w + x] as u32;
                }
            }
            *cell = sum as f32 / ((x1 - x0) * (y1 - y0)) as f32;
        }
    }
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            if cells[y * 9 + x] > cells[y * 9 + x + 1] {
                hash |= 1 << (y * 8 + x);
            }
        }
    }
    hash
}

const EDGE_T: i32 = 40;

fn is_edge(px: &[u8], w: usize, x: usize, y: usize) -> bool {
    let p = px[y * w + x] as i32;
    (p - px[y * w + x + 1] as i32).abs() + (p - px[(y + 1) * w + x] as i32).abs() > EDGE_T
}

/// 有效内容：横纵梯度绝对值之和超过阈值的像素占比。
pub fn content_score(px: &[u8], w: usize, h: usize) -> f32 {
    let mut edges = 0usize;
    for y in 0..h.saturating_sub(1) {
        for x in 0..w.saturating_sub(1) {
            if is_edge(px, w, x, y) {
                edges += 1;
            }
        }
    }
    edges as f32 / ((w.max(2) - 1) * (h.max(2) - 1)) as f32
}

/// 边缘位图（w×h 位）。
pub fn edge_bits(px: &[u8], w: usize, h: usize) -> Vec<u64> {
    let mut bits = vec![0u64; (w * h).div_ceil(64)];
    for y in 0..h.saturating_sub(1) {
        for x in 0..w.saturating_sub(1) {
            if is_edge(px, w, x, y) {
                let i = y * w + x;
                bits[i / 64] |= 1 << (i % 64);
            }
        }
    }
    bits
}

fn bit(b: &[u64], i: usize) -> bool {
    (b[i / 64] >> (i % 64)) & 1 == 1
}

/// `a` 的边缘有多大比例落在 `b` 的边缘（3×3 邻域）内。任一为空返回 0。
pub fn contained(a: &[u64], b: &[u64]) -> f32 {
    let (w, h) = (SAMPLE_W, SAMPLE_H);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (mut tot, mut hit) = (0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if !bit(a, y * w + x) {
                continue;
            }
            tot += 1;
            let near = (y.saturating_sub(1)..=(y + 1).min(h - 1))
                .any(|yy| (x.saturating_sub(1)..=(x + 1).min(w - 1)).any(|xx| bit(b, yy * w + xx)));
            if near {
                hit += 1;
            }
        }
    }
    if tot == 0 {
        1.0
    } else {
        hit as f32 / tot as f32
    }
}

/// 切区间（返回每个区间的样本下标范围）。
fn split(samples: &[Sample], cfg: &KeyframeConfig, scene_cuts: &[f64]) -> Vec<(usize, usize)> {
    let n = samples.len();
    let mut cuts: Vec<usize> = vec![0];
    let mut anchor = 0usize;
    let mut i = 1;
    while i < n {
        let far = |j: usize| hamming(samples[j].hash, samples[anchor].hash) >= cfg.cut_hamming;
        if far(i) && (i..(i + 1 + cfg.confirm_samples).min(n)).all(far) {
            cuts.push(i);
            anchor = i;
        }
        i += 1;
    }
    // 场景检测切点，去掉与已有切点过近的。
    for &t in scene_cuts {
        let Some(idx) = samples.iter().position(|s| s.time >= t) else {
            continue;
        };
        let near = cuts
            .iter()
            .any(|&c| (samples[c].time - samples[idx].time).abs() < cfg.scene_dedupe_secs);
        if !near {
            cuts.push(idx);
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut ranges: Vec<(usize, usize)> = cuts
        .iter()
        .enumerate()
        .map(|(k, &c)| (c, cuts.get(k + 1).copied().unwrap_or(n)))
        .collect();
    // 过短区间并入前一个。
    let dur = |r: &(usize, usize)| samples[r.1 - 1].time - samples[r.0].time + cfg.interval_secs;
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in ranges.drain(..) {
        match merged.last_mut() {
            Some(last) if dur(&r) < cfg.min_interval_secs => last.1 = r.1,
            _ => merged.push(r),
        }
    }
    if merged.len() > 1 && dur(&merged[0]) < cfg.min_interval_secs {
        let first = merged.remove(0);
        merged[0].0 = first.0;
    }
    merged
}

fn best(samples: &[Sample], r: (usize, usize)) -> usize {
    (r.0..r.1)
        .max_by(|&a, &b| {
            samples[a]
                .content
                .total_cmp(&samples[b].content)
                .then(a.cmp(&b))
        })
        .unwrap_or(r.0)
}

/// 选帧主函数（纯函数，便于测试）。`speech_units_in(a, b)` 返回该时段转写字/词数。
pub fn select(
    samples: &[Sample],
    duration: f64,
    cfg: &KeyframeConfig,
    scene_cuts: &[f64],
    speech_units_in: &dyn Fn(f64, f64) -> usize,
) -> Vec<Keyframe> {
    if samples.is_empty() {
        return Vec::new();
    }
    let t_end = |i: usize| samples.get(i).map(|s| s.time).unwrap_or(duration);
    let mut ranges = split(samples, cfg, scene_cuts);
    // 讲话稀少的长区间细分。
    let mut fine = Vec::new();
    let step = ((cfg.sparse_split_secs / cfg.interval_secs).round() as usize).max(1);
    for r in ranges.drain(..) {
        let (a, b) = (samples[r.0].time, t_end(r.1));
        let mins = ((b - a) / 60.0).max(1e-9);
        let sparse = (speech_units_in(a, b) as f64) / mins < cfg.sparse_units_per_min;
        if sparse && b - a > cfg.sparse_split_secs {
            let mut s = r.0;
            while s < r.1 {
                fine.push((s, (s + step).min(r.1)));
                s += step;
            }
        } else {
            fine.push(r);
        }
    }
    // 每区间取最佳帧；相邻且代表帧相近的合并。
    let mut picks: Vec<(usize, (usize, usize))> = Vec::new();
    for r in fine {
        let b = best(samples, r);
        if let Some(last) = picks.last_mut() {
            if hamming(samples[last.0].hash, samples[b].hash) <= cfg.merge_hamming {
                let r2 = (last.1 .0, r.1);
                *last = (best(samples, r2), r2);
                continue;
            }
        }
        picks.push((b, r));
    }
    // 逐步出现（PPT 动画 / 板书续写）：前一帧被后一帧包含，只留后一帧，区间并过去。
    let mut built: Vec<(usize, (usize, usize))> = Vec::new();
    for p in picks {
        if let Some(last) = built.last_mut() {
            if contained(&samples[last.0].edges, &samples[p.0].edges) >= cfg.contain_ratio {
                *last = (p.0, (last.1 .0, p.1 .1));
                continue;
            }
        }
        built.push(p);
    }
    let picks = built;
    // 去重（翻回同一页）。
    let mut kept: Vec<(usize, (usize, usize))> = Vec::new();
    for p in picks {
        // 哈希相近且（有边缘信息时）内容也被已选帧包含才算重复：避免人占画面大头时把新写的板书当重复。
        let dup = |k: &(usize, (usize, usize))| {
            hamming(samples[k.0].hash, samples[p.0].hash) <= cfg.merge_hamming
                && (samples[p.0].edges.is_empty()
                    || contained(&samples[p.0].edges, &samples[k.0].edges) >= cfg.contain_ratio)
        };
        if !kept.iter().any(dup) {
            kept.push(p);
        }
    }
    // 不足 min_frames：反复把最长的区间对半分，各取最佳帧（短视频保底）。
    while kept.len() < cfg.min_frames {
        let Some((idx, _)) = kept
            .iter()
            .enumerate()
            .filter(|(_, k)| k.1 .1 - k.1 .0 >= 2)
            .max_by_key(|(_, k)| k.1 .1 - k.1 .0)
        else {
            break;
        };
        let (a, b) = kept[idx].1;
        let m = (a + b) / 2;
        kept[idx] = (best(samples, (a, m)), (a, m));
        kept.insert(idx + 1, (best(samples, (m, b)), (m, b)));
    }
    let cap = ((duration / 3600.0 * cfg.max_per_hour).ceil() as usize).max(cfg.min_frames);
    if kept.len() > cap {
        kept.sort_by(|a, b| samples[b.0].content.total_cmp(&samples[a.0].content));
        kept.truncate(cap);
        kept.sort_by_key(|k| k.0);
    }
    kept.iter()
        .enumerate()
        .map(|(n, &(i, r))| Keyframe {
            n,
            time: samples[i].time,
            start: samples[r.0].time,
            end: t_end(r.1),
            content: samples[i].content,
            hash: samples[i].hash,
        })
        .collect()
}

/// 选帧结果缓存（`<cache>/keyframes.json`），参数变化即重算。
#[derive(Serialize, Deserialize)]
struct Cached {
    config: KeyframeConfig,
    frames: Vec<Keyframe>,
}

pub const CACHE_FILE: &str = "keyframes.json";

/// 读取缓存的板书帧；没有或参数不同则对 `video` 重新选帧并写缓存。
pub async fn load_or_compute(
    ctx: &FfmpegContext,
    video: &Path,
    cache_dir: &Path,
    cfg: &KeyframeConfig,
    transcript: Option<&crate::models::Transcript>,
) -> Result<Vec<Keyframe>, AvrError> {
    let path = cache_dir.join(CACHE_FILE);
    if let Some(c) = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Cached>(&b).ok())
    {
        if c.config == *cfg {
            return Ok(c.frames);
        }
    }
    let samples = sample_video(ctx, video, cfg.interval_secs, cache_dir).await?;
    let duration = crate::ffmpeg::probe_duration(ctx, video)
        .await
        .unwrap_or(samples.len() as f64 * cfg.interval_secs);
    let scenes: Vec<f64> = crate::ffmpeg::detect_scenes(ctx, video, 0.3)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let units = |a: f64, b: f64| speech_units_between(transcript, a, b);
    let frames = select(&samples, duration, cfg, &scenes, &units);
    let body = serde_json::to_vec(&Cached {
        config: cfg.clone(),
        frames: frames.clone(),
    })
    .map_err(|e| AvrError::io(std::io::Error::other(e), "写选帧缓存失败"))?;
    crate::jobs::atomic_write(&path, &body)?;
    Ok(frames)
}

/// `[a, b)` 内（与之相交的非疑似幻觉句）的字 / 词数。
pub fn speech_units_between(t: Option<&crate::models::Transcript>, a: f64, b: f64) -> usize {
    t.map(|t| {
        t.segments
            .iter()
            .filter(|s| s.suspect.is_none() && s.start < b && s.end > a)
            .map(|s| crate::asr::postprocess::speech_units(&s.text))
            .sum()
    })
    .unwrap_or(0)
}

/// 转写是否稀少：全片每分钟字 / 词数低于 `cfg.sparse_units_per_min`。
pub fn is_sparse(
    t: Option<&crate::models::Transcript>,
    duration: f64,
    cfg: &KeyframeConfig,
) -> bool {
    if duration <= 0.0 {
        return false;
    }
    (speech_units_between(t, 0.0, f64::MAX) as f64) / (duration / 60.0) < cfg.sparse_units_per_min
}

/// JPEG 质量（1–100）→ ffmpeg mjpeg 的 `-q:v`（2 最好、31 最差，线性近似）。
pub fn jpeg_qscale(quality: u8) -> u32 {
    let q = quality.clamp(1, 100) as u32;
    2 + (100 - q) * 29 / 99
}

/// 用 ffmpeg 一次抽出全部采样帧（灰度 rawvideo 写临时文件）。
pub async fn sample_video(
    ctx: &FfmpegContext,
    video: &Path,
    interval_secs: f64,
    tmp_dir: &Path,
) -> Result<Vec<Sample>, AvrError> {
    std::fs::create_dir_all(tmp_dir).map_err(|e| AvrError::io(e, "无法创建选帧临时目录"))?;
    let out = tmp_dir.join("keyframe_samples.gray");
    let req = ProcessRequest {
        program: ctx.ffmpeg.clone(),
        args: vec![
            "-y".into(),
            "-loglevel".into(),
            "error".into(),
            "-i".into(),
            video.display().to_string(),
            "-vf".into(),
            format!("fps=1/{interval_secs},scale={SAMPLE_W}:{SAMPLE_H},format=gray"),
            "-f".into(),
            "rawvideo".into(),
            out.display().to_string(),
        ],
        cwd: None,
        timeout: Some(Duration::from_secs(1800)),
        expected_outputs: vec![out.clone()],
    };
    run(&req).await?;
    let bytes = std::fs::read(&out).map_err(|e| AvrError::io(e, "读取选帧采样失败"))?;
    let _ = std::fs::remove_file(&out);
    Ok(samples_from_raw(&bytes, interval_secs))
}

/// rawvideo 字节 → 采样帧；第 k 帧时间为 k×interval（ffmpeg fps 滤镜从 0 开始）。
pub fn samples_from_raw(bytes: &[u8], interval_secs: f64) -> Vec<Sample> {
    bytes
        .as_chunks::<{ SAMPLE_W * SAMPLE_H }>()
        .0
        .iter()
        .enumerate()
        .map(|(k, px)| Sample {
            time: k as f64 * interval_secs,
            hash: dhash(px, SAMPLE_W, SAMPLE_H),
            content: content_score(px, SAMPLE_W, SAMPLE_H),
            edges: edge_bits(px, SAMPLE_W, SAMPLE_H),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: f64, hash: u64, content: f32) -> Sample {
        Sample {
            time: t,
            hash,
            content,
            edges: Vec::new(),
        }
    }
    fn cfg0() -> KeyframeConfig {
        KeyframeConfig {
            min_frames: 0,
            max_per_hour: 1e6,
            ..Default::default()
        }
    }
    fn none(_: f64, _: f64) -> usize {
        1000
    }
    const A: u64 = 0;
    const B: u64 = u64::MAX; // 与 A 距离 64
    const A2: u64 = 0b111; // 与 A 距离 3（逐笔书写 / 动画的小变化）

    #[test]
    fn picks_fullest_frame_per_interval() {
        // 两页：第一页逐渐写满（内容递增）后擦掉换第二页。
        let mut v: Vec<Sample> = (0..10).map(|i| s(i as f64 * 2.0, A, i as f32)).collect();
        v.extend((10..20).map(|i| s(i as f64 * 2.0, B, 1.0)));
        let k = select(&v, 40.0, &cfg0(), &[], &none);
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].time, 18.0, "取写满时（擦掉前）那张");
        assert_eq!((k[1].start, k[1].end), (20.0, 40.0));
    }

    #[test]
    fn small_changes_stay_in_one_interval() {
        let v: Vec<Sample> = (0..20)
            .map(|i| s(i as f64 * 2.0, if i % 2 == 0 { A } else { A2 }, 1.0))
            .collect();
        assert_eq!(select(&v, 40.0, &cfg0(), &[], &none).len(), 1);
    }

    #[test]
    fn brief_occlusion_or_shake_does_not_cut() {
        let mut v: Vec<Sample> = (0..20).map(|i| s(i as f64 * 2.0, A, 1.0)).collect();
        v[7].hash = B; // 一闪而过
        v[8].hash = B;
        let k = select(&v, 40.0, &cfg0(), &[], &none);
        assert_eq!(k.len(), 1);
        assert_ne!(k[0].time, 14.0);
    }

    #[test]
    fn returning_to_same_slide_is_deduped() {
        let mut v = Vec::new();
        for (p, h) in [A, B, A].iter().enumerate() {
            v.extend((0..10).map(|i| s((p * 10 + i) as f64 * 2.0, *h, 1.0)));
        }
        assert_eq!(select(&v, 60.0, &cfg0(), &[], &none).len(), 2);
    }

    #[test]
    fn sparse_speech_splits_long_static_interval() {
        let v: Vec<Sample> = (0..150).map(|i| s(i as f64 * 2.0, A, i as f32)).collect();
        let cfg = KeyframeConfig {
            merge_hamming: 0,
            ..cfg0()
        };
        // 讲话密：一个区间；讲话稀：按 60 秒细分（但画面没变，去重后仍只剩不同画面）。
        assert_eq!(select(&v, 300.0, &cfg, &[], &none).len(), 1);
        let mut w = v.clone();
        for (i, x) in w.iter_mut().enumerate() {
            x.hash = (1u64 << ((i / 30) * 2)) - 1; // 慢漂移：60 秒多 2 位，累计 8 位 < 切分阈值 14
        }
        assert_eq!(select(&w, 300.0, &cfg, &[], &none).len(), 1);
        assert!(select(&w, 300.0, &cfg, &[], &|_, _| 0).len() >= 4);
    }

    #[test]
    fn cap_per_hour_with_min_frames() {
        // 100 个互不相同的画面，时长 10 分钟 → 上限 max(12, ceil(40/6)) = 12。
        let v: Vec<Sample> = (0..100)
            .map(|i| {
                s(
                    i as f64 * 6.0,
                    (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                    i as f32,
                )
            })
            .collect();
        let cfg = KeyframeConfig {
            min_interval_secs: 0.0,
            confirm_samples: 0,
            min_frames: 12,
            max_per_hour: 40.0,
            ..cfg0()
        };
        let k = select(&v, 600.0, &cfg, &[], &none);
        assert_eq!(k.len(), 12);
        assert!(k.windows(2).all(|w| w[0].time < w[1].time));
        assert!(k.iter().all(|f| f.content >= 88.0), "按有效内容取前 12");
    }

    #[test]
    fn scene_cuts_merged_and_deduped() {
        let v: Vec<Sample> = (0..30).map(|i| s(i as f64 * 2.0, A, 1.0)).collect();
        let cfg = KeyframeConfig {
            merge_hamming: 0,
            ..cfg0()
        };
        // 画面哈希不变时，场景切点产生的区间会因代表帧相同被合并回去。
        assert_eq!(select(&v, 60.0, &cfg, &[20.0, 21.0], &none).len(), 1);
    }

    #[test]
    fn dhash_and_content_basic() {
        let flat = vec![128u8; SAMPLE_W * SAMPLE_H];
        assert_eq!(content_score(&flat, SAMPLE_W, SAMPLE_H), 0.0);
        let stripes: Vec<u8> = (0..SAMPLE_W * SAMPLE_H)
            .map(|i| if (i % SAMPLE_W) % 4 < 2 { 0 } else { 255 })
            .collect();
        assert!(content_score(&stripes, SAMPLE_W, SAMPLE_H) > 0.4);
        assert_eq!(dhash(&flat, SAMPLE_W, SAMPLE_H), 0);
        assert_eq!(samples_from_raw(&flat, 2.0).len(), 1);
    }

    #[test]
    fn min_frames_fills_by_splitting_longest() {
        let v: Vec<Sample> = (0..60).map(|i| s(i as f64 * 2.0, A, i as f32)).collect();
        let cfg = KeyframeConfig {
            min_frames: 4,
            ..Default::default()
        };
        let k = select(&v, 120.0, &cfg, &[], &none);
        assert_eq!(k.len(), 4);
        assert!(k.windows(2).all(|w| w[0].end <= w[1].start + 1e-9));
    }

    #[test]
    fn build_up_animation_keeps_only_final_state() {
        // 第一步上方有字；第二步（动画）在下方再出现内容：哈希变化大，但前一帧被后一帧包含。
        let mut top = vec![255u8; SAMPLE_W * SAMPLE_H];
        for y in 10..20 {
            for x in (10..150).step_by(4) {
                top[y * SAMPLE_W + x] = 0;
            }
        }
        let mut full = top.clone();
        for y in 50..90 {
            for x in 0..SAMPLE_W {
                // 左亮右暗的渐变（改变 dHash）叠加竖线（产生边缘）。
                full[y * SAMPLE_W + x] = if x % 3 == 0 {
                    0
                } else {
                    (255 - x * 3 / 2) as u8
                };
            }
        }
        let (et, ef) = (
            edge_bits(&top, SAMPLE_W, SAMPLE_H),
            edge_bits(&full, SAMPLE_W, SAMPLE_H),
        );
        assert!(contained(&et, &ef) > 0.95);
        assert!(contained(&ef, &et) < 0.5);
        let mut raw = Vec::new();
        for _ in 0..10 {
            raw.extend_from_slice(&top);
        }
        for _ in 0..10 {
            raw.extend_from_slice(&full);
        }
        let v = samples_from_raw(&raw, 2.0);
        assert!(
            hamming(v[0].hash, v[19].hash) >= cfg0().cut_hamming,
            "测试前提：哈希变化足以切分"
        );
        let k = select(&v, 40.0, &cfg0(), &[], &none);
        assert_eq!(k.len(), 1);
        assert!(k[0].time >= 20.0);
        assert_eq!((k[0].start, k[0].end), (0.0, 40.0));
    }

    #[test]
    fn jpeg_quality_mapping() {
        assert_eq!(jpeg_qscale(100), 2);
        assert_eq!(jpeg_qscale(85), 6);
        assert_eq!(jpeg_qscale(1), 31);
        assert_eq!(jpeg_qscale(0), 31);
    }

    #[test]
    fn sparse_detection_ignores_suspects() {
        use crate::models::{SuspectReason, Transcript, TranscriptSegment, TranscriptSource};
        let seg = |t: &str, suspect| TranscriptSegment {
            start: 0.0,
            end: 5.0,
            text: t.into(),
            suspect,
        };
        let cfg = KeyframeConfig::default();
        let dense = Transcript {
            language: "zh".into(),
            source: TranscriptSource::Asr,
            segments: vec![seg(&"字".repeat(100), None)],
        };
        assert!(!is_sparse(Some(&dense), 60.0, &cfg));
        assert!(is_sparse(Some(&dense), 600.0, &cfg), "10 分钟 100 字");
        let fake = Transcript {
            segments: vec![seg(&"字".repeat(100), Some(SuspectReason::Repeat))],
            ..dense.clone()
        };
        assert!(is_sparse(Some(&fake), 60.0, &cfg));
        assert!(is_sparse(None, 60.0, &cfg));
    }
}
