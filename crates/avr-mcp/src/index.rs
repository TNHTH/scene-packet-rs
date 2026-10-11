//! P3-2 按需读取：analyze 只返回摘要索引，正文与关键帧通过只读工具 / resource 取回。
//!
//! - [`summary_index`]：元数据 + 分段（section）列表（时间范围、一句话概要、帧编号）。
//! - [`read_transcript`]：按时间范围取转写原文，不透明 cursor 分页。
//! - [`frame_ref`]：按编号取关键帧的时间与 resource URI（`avr://job/{media_key}/frame/{n}`）。

use avr_core::media::MediaKey;
use avr_core::models::{AnalyzeResult, TranscriptSegment, TranscriptSource};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 一个 section 最多覆盖的时长（秒）与分段数，先到为准。
pub const SECTION_MAX_SECS: f64 = 60.0;
pub const SECTION_MAX_SEGMENTS: usize = 20;
/// 一句话概要的最大字符数。
pub const SUMMARY_CHARS: usize = 40;
/// read_transcript 单页默认最大字符数。
pub const DEFAULT_PAGE_CHARS: usize = 4000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub id: usize,
    pub start: f64,
    pub end: f64,
    /// 该 section 第一个分段在全文中的下标。
    pub first_segment: usize,
    pub segment_count: usize,
    /// 一句话概要：首个分段文本截断到 SUMMARY_CHARS 个字符。
    pub summary: String,
    /// 关键帧编号（与 section id 相同）。
    pub frame: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameRef {
    pub n: usize,
    pub time: f64,
    pub resource_uri: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexedSegment {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspect: Option<avr_core::models::SuspectReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptPage {
    pub media_key: String,
    pub segments: Vec<IndexedSegment>,
    #[serde(rename = "nextCursor", skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    BadCursor,
    NoTranscript,
    FrameOutOfRange(usize),
    BadUri,
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadCursor => f.write_str("cursor 无效或与本次查询不匹配"),
            Self::NoTranscript => f.write_str("没有转写"),
            Self::FrameOutOfRange(n) => write!(f, "帧编号 {n} 超出范围"),
            Self::BadUri => f.write_str("resource URI 格式应为 avr://job/{media_key}/frame/{n}"),
        }
    }
}

impl std::error::Error for IndexError {}

fn segments(result: &AnalyzeResult) -> &[TranscriptSegment] {
    result
        .transcript
        .as_ref()
        .map(|t| t.segments.as_slice())
        .unwrap_or_default()
}

fn summarize(text: &str) -> String {
    let mut s: String = text.chars().take(SUMMARY_CHARS).collect();
    if text.chars().count() > SUMMARY_CHARS {
        s.push('…');
    }
    s
}

/// 把分段按时长 / 数量切成 section。
pub fn sections(result: &AnalyzeResult) -> Vec<Section> {
    let segs = segments(result);
    let mut out: Vec<Section> = Vec::new();
    // 疑似幻觉句不进索引（原文仍可经 read_transcript / full 取回）。
    for (i, seg) in segs.iter().enumerate().filter(|(_, s)| s.suspect.is_none()) {
        let open = out.last().filter(|s| {
            s.segment_count < SECTION_MAX_SEGMENTS && seg.end - s.start <= SECTION_MAX_SECS
        });
        if open.is_some() {
            let s = out.last_mut().expect("checked");
            s.segment_count += 1;
            s.end = s.end.max(seg.end);
        } else {
            let id = out.len();
            out.push(Section {
                id,
                start: seg.start,
                end: seg.end,
                first_segment: i,
                segment_count: 1,
                summary: summarize(&seg.text),
                frame: id,
            });
        }
    }
    out
}

pub fn frame_uri(media_key: &str, n: usize) -> String {
    format!("avr://job/{media_key}/frame/{n}")
}

/// 解析 `avr://job/{media_key}/frame/{n}`；media_key 经 [`MediaKey::parse`] 白名单校验。
pub fn parse_frame_uri(uri: &str) -> Result<(MediaKey, usize), IndexError> {
    let rest = uri.strip_prefix("avr://job/").ok_or(IndexError::BadUri)?;
    let (key, n) = rest.rsplit_once("/frame/").ok_or(IndexError::BadUri)?;
    let key = MediaKey::parse(key).map_err(|_| IndexError::BadUri)?;
    Ok((key, n.parse().map_err(|_| IndexError::BadUri)?))
}

/// 帧 n 的时间：对应 section 首个分段的中点（必然落在该 section 内）。
pub fn frame_ref(result: &AnalyzeResult, n: usize) -> Result<FrameRef, IndexError> {
    let secs = sections(result);
    let s = secs.get(n).ok_or(IndexError::FrameOutOfRange(n))?;
    let seg = &segments(result)[s.first_segment];
    Ok(FrameRef {
        n,
        time: (seg.start + seg.end) / 2.0,
        resource_uri: frame_uri(&result.media_key, n),
    })
}

fn source_str(s: TranscriptSource) -> &'static str {
    match s {
        TranscriptSource::Subtitle => "subtitle",
        TranscriptSource::Asr => "asr",
    }
}

/// analyze 的默认返回：元数据 + section 索引 + 帧列表，不含正文。
pub fn summary_index(result: &AnalyzeResult) -> Value {
    let trans = result.transcript.as_ref();
    let secs = sections(result);
    let frames: Vec<FrameRef> = (0..secs.len())
        .filter_map(|n| frame_ref(result, n).ok())
        .collect();
    json!({
        "media_key": result.media_key,
        "title": result.video.title,
        "platform": result.video.platform,
        "duration": result.video.duration,
        "video_id": result.video.id,
        "source": trans.map(|t| source_str(t.source)),
        "language": trans.map(|t| t.language.clone()),
        "truncated": result.truncated,
        "transcript_path": result.transcript_path,
        "transcript_uri": result.transcript_uri,
        "text_file": result.transcript_path,
        "cache_dir": result.cache_dir,
        "segment_count": segments(result).len(),
        "sections": secs,
        "frames": frames,
        "next": "用 read_transcript(media_key, start, end) 取原文；用 get_frame(media_key, n) 或读取 frames[].resource_uri 取关键帧。",
    })
}

#[derive(Serialize, Deserialize)]
struct CursorData {
    k: String,
    s: f64,
    e: f64,
    i: usize,
}

fn encode_cursor(c: &CursorData) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(c).expect("serialize cursor"))
}

fn decode_cursor(s: &str) -> Result<CursorData, IndexError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| IndexError::BadCursor)?;
    serde_json::from_slice(&bytes).map_err(|_| IndexError::BadCursor)
}

/// 取与 `[start, end)` 有交集的分段原文，单页不超过 `max_chars` 个字符（至少 1 段）。
/// `cursor` 来自上一页的 `nextCursor`，必须与同一 media_key / 时间范围配套使用。
pub fn read_transcript(
    result: &AnalyzeResult,
    start: f64,
    end: f64,
    cursor: Option<&str>,
    max_chars: usize,
) -> Result<TranscriptPage, IndexError> {
    let segs = segments(result);
    if result.transcript.is_none() {
        return Err(IndexError::NoTranscript);
    }
    let from = match cursor {
        None => 0,
        Some(c) => {
            let c = decode_cursor(c)?;
            if c.k != result.media_key || c.s != start || c.e != end || c.i > segs.len() {
                return Err(IndexError::BadCursor);
            }
            c.i
        }
    };
    let mut out = Vec::new();
    let mut chars = 0;
    let mut next = None;
    for (i, seg) in segs.iter().enumerate().skip(from) {
        // 与 [start, end) 有交集；零时长分段只要起点落在区间内也算。
        let overlaps = seg.start < end && (seg.end > start || seg.start >= start);
        if !overlaps {
            continue;
        }
        let n = seg.text.chars().count();
        if !out.is_empty() && chars + n > max_chars {
            next = Some(encode_cursor(&CursorData {
                k: result.media_key.clone(),
                s: start,
                e: end,
                i,
            }));
            break;
        }
        chars += n;
        out.push(IndexedSegment {
            index: i,
            start: seg.start,
            end: seg.end,
            text: seg.text.clone(),
            suspect: seg.suspect,
        });
    }
    Ok(TranscriptPage {
        media_key: result.media_key.clone(),
        segments: out,
        next_cursor: next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_uri_roundtrip() {
        let u = frame_uri("local-abc", 3);
        let (k, n) = parse_frame_uri(&u).unwrap();
        assert_eq!((k.as_str(), n), ("local-abc", 3));
        assert!(parse_frame_uri("avr://job/a/b/frame/1").is_err());
        assert!(parse_frame_uri("avr://job/../frame/1").is_err());
        assert!(parse_frame_uri("avr://job/..%2F/frame/1").is_err());
        assert!(parse_frame_uri("http://x").is_err());
        assert!(parse_frame_uri("avr://job/k/frame/x").is_err());
    }

    #[test]
    fn suspects_skipped_in_index_but_readable() {
        use avr_core::models::SuspectReason;
        let mut r = crate::payload::tests::sample("真话");
        let t = r.transcript.as_mut().unwrap();
        let mut dup = t.segments[0].clone();
        dup.start = 1.0;
        dup.end = 2.0;
        dup.suspect = Some(SuspectReason::Repeat);
        t.segments.push(dup);
        let secs = sections(&r);
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].segment_count, 1);
        assert_eq!(secs[0].end, 1.0);
        let page = read_transcript(&r, 0.0, 10.0, None, 4000).unwrap();
        assert_eq!(page.segments.len(), 2, "原文仍可取回");
        assert_eq!(page.segments[1].suspect, Some(SuspectReason::Repeat));
        let c = crate::payload::compact_mcp_payload(&r, 3000);
        assert_eq!(c["text"], "真话");
        assert_eq!(c["sample_segments"].as_array().unwrap().len(), 1);
        let f = crate::payload::full_mcp_payload(&r);
        assert_eq!(f["segments"][1]["suspect"], "repeat");
    }
}
