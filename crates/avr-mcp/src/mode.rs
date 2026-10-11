//! analyze_media 的返回模式选择（纯函数）。

use avr_core::models::AnalyzeResult;
use serde_json::Value;

use crate::index::summary_index;
use crate::payload::{compact_mcp_payload, full_mcp_payload};
use crate::tokens::estimate_tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseMode {
    /// 仅显式 `response_mode=auto`：短视频直接 full，否则 index。
    Auto,
    Index,
    Compact,
    Full,
}

impl ResponseMode {
    /// 解析客户端传入的 response_mode。未指定 / 空串 / 未知值都是 Compact（与 P3-2 之前一致）。
    pub fn parse(s: Option<&str>) -> Self {
        match s.map(str::trim).unwrap_or("") {
            "auto" => Self::Auto,
            "full" => Self::Full,
            "index" => Self::Index,
            _ => Self::Compact,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Index => "index",
            Self::Compact => "compact",
            Self::Full => "full",
        }
    }
}

/// 转写原文的估算 token 数。
pub fn transcript_tokens(result: &AnalyzeResult) -> usize {
    result
        .transcript
        .as_ref()
        .map(|t| estimate_tokens(&t.full_text()))
        .unwrap_or(0)
}

/// 按模式生成返回 JSON，并给出实际采用的模式（Auto 会解析成 Full 或 Index）。
pub fn render(
    result: &AnalyzeResult,
    mode: ResponseMode,
    max_inline: usize,
    auto_full_max_tokens: usize,
) -> (Value, ResponseMode) {
    let resolved = match mode {
        ResponseMode::Auto if transcript_tokens(result) <= auto_full_max_tokens => {
            ResponseMode::Full
        }
        ResponseMode::Auto => ResponseMode::Index,
        m => m,
    };
    let v = match resolved {
        ResponseMode::Full => full_mcp_payload(result),
        ResponseMode::Compact => compact_mcp_payload(result, max_inline),
        _ => summary_index(result),
    };
    (v, resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::tests::sample as result_with;

    #[test]
    fn parse_defaults_to_compact() {
        assert_eq!(ResponseMode::parse(None), ResponseMode::Compact);
        assert_eq!(ResponseMode::parse(Some("")), ResponseMode::Compact);
        assert_eq!(ResponseMode::parse(Some("weird")), ResponseMode::Compact);
        assert_eq!(ResponseMode::parse(Some(" auto ")), ResponseMode::Auto);
        assert_eq!(ResponseMode::parse(Some("index")), ResponseMode::Index);
        assert_eq!(ResponseMode::parse(Some("full")), ResponseMode::Full);
    }

    #[test]
    fn auto_switches_on_threshold() {
        let r = result_with("你好世界"); // 4 tokens
        assert_eq!(
            render(&r, ResponseMode::Auto, 3000, 4).1,
            ResponseMode::Full
        );
        assert_eq!(
            render(&r, ResponseMode::Auto, 3000, 3).1,
            ResponseMode::Index
        );
        assert_eq!(
            full_mcp_payload(&r),
            render(&r, ResponseMode::Auto, 3000, 4).0
        );
        assert_eq!(summary_index(&r), render(&r, ResponseMode::Auto, 3000, 3).0);
        // 显式模式不受阈值影响。
        assert_eq!(
            render(&r, ResponseMode::Compact, 3000, 0).1,
            ResponseMode::Compact
        );
        assert_eq!(
            render(&r, ResponseMode::Full, 3000, 0).1,
            ResponseMode::Full
        );
        assert_eq!(
            render(&r, ResponseMode::Index, 3000, 99).1,
            ResponseMode::Index
        );
    }
}
