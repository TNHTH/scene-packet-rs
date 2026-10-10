//! SRT/VTT/JSON 时间轴解析。

use std::path::Path;

use regex::Regex;
use std::sync::LazyLock;

use crate::models::{Transcript, TranscriptSegment, TranscriptSource};

static VTT_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d{2}:\d{2}:\d{2}[.,]\d{3})\s*-->\s*(\d{2}:\d{2}:\d{2}[.,]\d{3})").unwrap()
});

static SRT_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d{2}:\d{2}:\d{2},\d{3})\s*-->\s*(\d{2}:\d{2}:\d{2},\d{3})").unwrap()
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

pub fn parse_vtt(content: &str, language: &str) -> Transcript {
    let mut segments = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut current_start: Option<f64> = None;
    let mut current_end: Option<f64> = None;
    let mut buffer: Vec<String> = Vec::new();

    let flush = |start: &mut Option<f64>,
                 end: &mut Option<f64>,
                 buffer: &mut Vec<String>,
                 segments: &mut Vec<TranscriptSegment>,
                 seen: &mut std::collections::HashSet<String>| {
        if let Some(s) = *start {
            if !buffer.is_empty() {
                let text = buffer.join(" ").trim().to_string();
                let text = strip_tags(&text);
                if !text.is_empty() && !seen.contains(&text) {
                    seen.insert(text.clone());
                    segments.push(TranscriptSegment {
                        start: s,
                        end: end.unwrap_or(s),
                        text,
                    });
                }
            }
        }
        buffer.clear();
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("WEBVTT")
            || line.starts_with("Kind:")
            || line.starts_with("Language:")
        {
            continue;
        }
        if let Some(m) = VTT_TIME.captures(line) {
            flush(
                &mut current_start,
                &mut current_end,
                &mut buffer,
                &mut segments,
                &mut seen,
            );
            current_start = Some(parse_ts(m.get(1).unwrap().as_str()));
            current_end = Some(parse_ts(m.get(2).unwrap().as_str()));
            continue;
        }
        if line.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        buffer.push(line.to_string());
    }
    flush(
        &mut current_start,
        &mut current_end,
        &mut buffer,
        &mut segments,
        &mut seen,
    );

    Transcript {
        language: language.to_string(),
        source: TranscriptSource::Subtitle,
        segments,
    }
}

pub fn parse_srt(content: &str, language: &str) -> Transcript {
    let mut segments = Vec::new();
    for block in content.trim().split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        if lines.len() < 2 {
            continue;
        }
        let (time_line, text_lines) = if lines[0].chars().all(|c| c.is_ascii_digit()) {
            (lines[1], &lines[2..])
        } else {
            (lines[0], &lines[1..])
        };
        if let Some(m) = SRT_TIME.captures(time_line) {
            let start = parse_srt_ts(m.get(1).unwrap().as_str());
            let end = parse_srt_ts(m.get(2).unwrap().as_str());
            let text = text_lines.join(" ").trim().to_string();
            if !text.is_empty() {
                segments.push(TranscriptSegment { start, end, text });
            }
        }
    }
    Transcript {
        language: language.to_string(),
        source: TranscriptSource::Subtitle,
        segments,
    }
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

fn parse_srt_ts(ts: &str) -> f64 {
    let parts: Vec<&str> = ts.split(':').collect();
    if parts.len() != 3 {
        return 0.0;
    }
    let h = parts[0].parse::<f64>().unwrap_or(0.0);
    let m = parts[1].parse::<f64>().unwrap_or(0.0);
    let rest: Vec<&str> = parts[2].split(',').collect();
    let s = rest[0].parse::<f64>().unwrap_or(0.0);
    let ms = rest
        .get(1)
        .and_then(|x| x.parse::<f64>().ok())
        .unwrap_or(0.0);
    h * 3600.0 + m * 60.0 + s + ms / 1000.0
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
