//! 外部工具标识，替代散落各处的硬编码字符串。

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolId {
    Ffmpeg,
    Ffprobe,
    YtDlp,
    Whisper,
}

impl ToolId {
    /// 所有可管理的工具，按界面展示顺序。
    pub const ALL: [ToolId; 4] = [Self::Ffmpeg, Self::Ffprobe, Self::YtDlp, Self::Whisper];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
            Self::YtDlp => "yt-dlp",
            Self::Whisper => "whisper",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// 该工具是否由给定的安装清单 id 提供（whisper 有 cpu / cublas 多个清单）。
    pub fn matches_manifest(&self, manifest_id: &str) -> bool {
        match self {
            Self::Whisper => manifest_id.starts_with("whisper"),
            _ => manifest_id == self.as_str(),
        }
    }
}

impl fmt::Display for ToolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_id_roundtrip_and_manifest_match() {
        for t in ToolId::ALL {
            assert_eq!(ToolId::parse(t.as_str()), Some(t));
        }
        assert_eq!(ToolId::parse("nope"), None);
        assert!(ToolId::Whisper.matches_manifest("whisper-cpp-cublas"));
        assert!(ToolId::YtDlp.matches_manifest("yt-dlp"));
        assert!(!ToolId::Ffmpeg.matches_manifest("ffprobe"));
    }
}
