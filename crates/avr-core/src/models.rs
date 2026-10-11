//! 数据模型。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration: f64,
    pub platform: Platform,
    pub webpage_url: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Bilibili,
    Douyin,
    /// 本地媒体文件（非在线平台）。
    Local,
}

impl Platform {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bilibili => "bilibili",
            Self::Douyin => "douyin",
            Self::Local => "local",
        }
    }

    /// 从缓存元数据中的字符串还原；未知值返回 None，由调用方决定兜底。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bilibili" => Some(Self::Bilibili),
            "douyin" => Some(Self::Douyin),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// ASR 后处理标记的疑似幻觉（时长与字数明显不匹配，见 `asr::postprocess`）。
    /// 为 false 时不序列化，旧缓存 / 字幕 JSON 不受影响。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suspect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Transcript {
    pub language: String,
    pub source: TranscriptSource,
    pub segments: Vec<TranscriptSegment>,
}

impl Transcript {
    pub fn full_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptSource {
    Subtitle,
    Asr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    /// 缓存格式版本，见 `cache::CACHE_SCHEMA`；旧缓存缺省为 0（视为未命中）。
    #[serde(default)]
    pub cache_schema: u32,
    pub app_version: String,
    pub asr_provider: Option<String>,
    pub asr_model: String,
    /// 生成转写时 ASR 引擎的版本指纹；字幕来源为空。
    #[serde(default)]
    pub asr_version: String,
    pub video_id: String,
    pub platform: String,
    pub source: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeResult {
    pub video: VideoInfo,
    pub transcript: Option<Transcript>,
    pub transcript_path: String,
    pub transcript_uri: String,
    pub cache_dir: String,
    pub media_key: String,
    pub truncated: bool,
}

pub const TRANSCRIPT_INLINE_LIMIT: usize = 12000;
pub const TRANSCRIPT_PREVIEW_LIMIT: usize = 3000;

pub fn format_inline_text(transcript: &Transcript, truncated: bool) -> String {
    let text = transcript.full_text();
    if truncated && text.len() > TRANSCRIPT_PREVIEW_LIMIT {
        format!(
            "{}\n\n...(内容已截断，请读取 transcript_path 获取全文后再总结)",
            truncate_utf8(&text, TRANSCRIPT_PREVIEW_LIMIT)
        )
    } else {
        text
    }
}

pub fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::Platform;

    #[test]
    fn platform_str_roundtrip_includes_local() {
        for p in [Platform::Bilibili, Platform::Douyin, Platform::Local] {
            assert_eq!(Platform::parse(p.as_str()), Some(p));
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(json, format!("\"{}\"", p.as_str()));
        }
        assert_eq!(Platform::parse("youtube"), None);
    }
}
