//! SRT/VTT/JSON 时间轴解析。

use std::path::Path;

use regex::Regex;
use std::sync::LazyLock;

use crate::models::{Transcript, TranscriptSegment, TranscriptSource};

static CUE_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^((?:\d{2,}:)?\d{2}:\d{2}[.,]\d{3})\s*-->\s*((?:\d{2,}:)?\d{2}:\d{2}[.,]\d{3})")
        .unwrap()
});

const ZH_LANG_PRIORITY: [&str; 4] = ["zh-Hans", "zh-CN", "zh", "ai-zh"];

pub fn pick_subtitle_lang(available: &[String]) -> Option<String> {
    for pref in ZH_LANG_PRIORITY {
        for key in available {
            if key.to_lowercase().starts_with(&pref.to_lowercase())
                || key.to_lowercase().contains(&pref.to_lowercase())
            {
                return Some(key.clone());
            }
        }
    }
    for key in available {
        if key.to_lowercase().contains("zh") {
            return Some(key.clone());
        }
    }
    available.first().cloned()
}

fn parse_cues(content: &str, language: &str) -> Transcript {
    let normalized = content
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut segments = Vec::new();
    for block in normalized.split("\n\n") {
        let lines: Vec<_> = block.lines().collect();
        if lines.first().is_some_and(|line| {
            line.starts_with("NOTE") || line.starts_with("STYLE") || line.starts_with("REGION")
        }) {
            continue;
        }
        if let Some((index, captures)) = lines
            .iter()
            .enumerate()
            .find_map(|(i, line)| CUE_TIME.captures(line.trim()).map(|c| (i, c)))
        {
            let start = parse_ts(&captures[1]);
            let end = parse_ts(&captures[2]);
            let text = strip_tags(&lines[index + 1..].join(" ")).trim().to_string();
            if !text.is_empty() && end >= start {
                segments.push(TranscriptSegment { start, end, text });
            }
        }
    }
    Transcript {
        language: language.into(),
        source: TranscriptSource::Subtitle,
        segments,
    }
}
pub fn parse_vtt(content: &str, language: &str) -> Transcript {
    parse_cues(content, language)
}
pub fn parse_srt(content: &str, language: &str) -> Transcript {
    parse_cues(content, language)
}

pub fn parse_srt_file(path: &Path, language: &str) -> std::io::Result<Transcript> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse_srt(&content, language))
}

pub fn segments_to_srt(transcript: &Transcript) -> String {
    let mut lines = Vec::new();
    for (i, seg) in transcript.segments.iter().enumerate() {
        lines.push((i + 1).to_string());
        lines.push(format!("{} --> {}", fmt_ts(seg.start), fmt_ts(seg.end)));
        lines.push(seg.text.clone());
        lines.push(String::new());
    }
    lines.join("\n")
}

fn strip_tags(s: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    re.replace_all(s, "")
        .replace("&amp;", "&")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .to_string()
}

fn parse_ts(ts: &str) -> f64 {
    let ts = ts.replace(',', ".");
    let parts: Vec<&str> = ts.split(':').collect();
    match parts.len() {
        3 => {
            let h = parts[0].parse::<f64>().unwrap_or(0.0);
            let m = parts[1].parse::<f64>().unwrap_or(0.0);
            let s = parts[2].parse::<f64>().unwrap_or(0.0);
            h * 3600.0 + m * 60.0 + s
        }
        2 => {
            let m = parts[0].parse::<f64>().unwrap_or(0.0);
            let s = parts[1].parse::<f64>().unwrap_or(0.0);
            m * 60.0 + s
        }
        _ => ts.parse().unwrap_or(0.0),
    }
}

fn fmt_ts(seconds: f64) -> String {
    let h = (seconds / 3600.0).floor() as u32;
    let m = ((seconds % 3600.0) / 60.0).floor() as u32;
    let s = seconds % 60.0;
    format!("{:02}:{:02}:{:06.3}", h, m, s).replace('.', ",")
}

pub fn format_timestamp(seconds: f64) -> String {
    let h = (seconds / 3600.0).floor() as u32;
    let m = ((seconds % 3600.0) / 60.0).floor() as u32;
    let s = (seconds % 60.0).floor() as u32;
    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sample_vtt() {
        let vtt = include_str!("../../../tests/fixtures/sample.zh.vtt");
        let t = parse_vtt(vtt, "zh");
        assert_eq!(t.segments.len(), 2);
        assert!(t.segments[0].text.contains("大家好"));
    }
}
