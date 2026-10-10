//! MCP 返回内容的 JSON 拼装（纯函数，不依赖 rmcp / Tauri，可在任意平台测试）。

use avr_core::models::{format_inline_text, truncate_utf8, AnalyzeResult, TranscriptSource};

/// 精简 payload：内联文本按 `max_inline` 截断，附 5 条示例分段。CLI / GUI 也复用。
pub fn compact_mcp_payload(result: &AnalyzeResult, max_inline: usize) -> serde_json::Value {
    let trans = result.transcript.as_ref();
    let sample = trans
        .map(|t| t.segments.iter().take(5).cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let text = trans
        .map(|t| format_inline_text(t, result.truncated))
        .unwrap_or_default();
    let inline = if text.len() > max_inline {
        format!("{}...", truncate_utf8(&text, max_inline))
    } else {
        text
    };
    serde_json::json!({
        "media_key": result.media_key,
        "title": result.video.title,
        "duration": result.video.duration,
        "platform": result.video.platform,
        "video_id": result.video.id,
        "source": trans.map(|t| match t.source { TranscriptSource::Subtitle => "subtitle", TranscriptSource::Asr => "asr" }),
        "language": trans.map(|t| t.language.clone()),
        "segment_count": trans.map(|t| t.segments.len()).unwrap_or(0),
        "transcript_path": result.transcript_path,
        "transcript_uri": result.transcript_uri,
        "text_file": result.transcript_path,
        "cache_dir": result.cache_dir,
        "truncated": result.truncated,
        "sample_segments": sample,
        "has_video": true,
        "text": inline,
    })
}

/// 完整 payload：包含全部分段。
pub fn full_mcp_payload(result: &AnalyzeResult) -> serde_json::Value {
    let trans = result.transcript.as_ref();
    serde_json::json!({
        "title": result.video.title,
        "platform": result.video.platform,
        "duration": result.video.duration,
        "video_id": result.video.id,
        "source": trans.map(|t| match t.source { TranscriptSource::Subtitle => "subtitle", TranscriptSource::Asr => "asr" }),
        "language": trans.map(|t| t.language.clone()),
        "truncated": result.truncated,
        "transcript_path": result.transcript_path,
        "transcript_uri": result.transcript_uri,
        "text_file": result.transcript_path,
        "cache_dir": result.cache_dir,
        "media_key": result.media_key,
        "segments": trans.map(|t| t.segments.clone()),
        "text": trans.map(|t| format_inline_text(t, result.truncated)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use avr_core::models::{Platform, Transcript, TranscriptSegment, VideoInfo};

    fn sample(text: &str) -> AnalyzeResult {
        AnalyzeResult {
            video: VideoInfo {
                id: "id".into(),
                title: "t".into(),
                url: "u".into(),
                duration: 1.0,
                platform: Platform::Local,
                webpage_url: "u".into(),
            },
            transcript: Some(Transcript {
                language: "zh".into(),
                source: TranscriptSource::Asr,
                segments: vec![TranscriptSegment {
                    start: 0.0,
                    end: 1.0,
                    text: text.into(),
                }],
            }),
            transcript_path: "p".into(),
            transcript_uri: "file:///p".into(),
            cache_dir: "c".into(),
            media_key: "k".into(),
            truncated: false,
        }
    }

    #[test]
    fn compact_payload_truncates_inline_text() {
        let v = compact_mcp_payload(&sample(&"字".repeat(100)), 10);
        assert_eq!(v["source"], "asr");
        assert!(v["text"].as_str().unwrap().ends_with("..."));
        assert_eq!(v["segment_count"], 1);
    }

    #[test]
    fn full_payload_has_segments() {
        let v = full_mcp_payload(&sample("hi"));
        assert_eq!(v["segments"].as_array().unwrap().len(), 1);
    }
}
