//! MCP 工具调用日志：每次调用一行 JSON，写 stderr 与 `<数据根>/logs/mcp-calls.jsonl`。
//! 不写 stdout（stdio 传输的协议通道）。

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ToolCallRecord {
    /// RFC3339 时间
    pub ts: String,
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_key: Option<String>,
    /// 参数摘要，如 `{"start":0,"end":30}`、`{"n":2}`、`{"source":"https://..."}`
    pub args: Value,
    /// 返回内容（所有 text content 的 UTF-8 字节数之和）
    pub bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mode: Option<String>,
    pub ok: bool,
}

/// 调用日志写入器。`path` 为 None 时只写 stderr。
#[derive(Debug, Clone, Default)]
pub struct CallLog {
    path: Option<PathBuf>,
}

impl CallLog {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// 默认位置：`<数据根>/logs/mcp-calls.jsonl`。
    pub fn in_logs_dir(logs_dir: &Path) -> Self {
        Self::new(Some(logs_dir.join("mcp-calls.jsonl")))
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 写一条记录；日志失败不影响工具调用本身。
    pub fn write(&self, rec: &ToolCallRecord) {
        let Ok(line) = serde_json::to_string(rec) else {
            return;
        };
        eprintln!("mcp-call {line}");
        if let Some(path) = &self.path {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

/// 字符串参数在日志里最多保留的字符数。
const ARG_STR_MAX: usize = 120;

/// 参数摘要：去掉 media_key（单独成字段）；长字符串截断，数组只记长度（如 `times` → `{"len":3}`），
/// cursor 只记是否存在。
pub fn summarize_args(args: Option<&Map<String, Value>>) -> Value {
    let mut out = Map::new();
    for (k, v) in args.into_iter().flatten() {
        if k == "media_key" {
            continue;
        }
        let v = match v {
            Value::String(_) if k == "cursor" => Value::Bool(true),
            Value::String(s) if s.chars().count() > ARG_STR_MAX => Value::String(format!(
                "{}…",
                s.chars().take(ARG_STR_MAX).collect::<String>()
            )),
            Value::Array(a) => serde_json::json!({ "len": a.len() }),
            other => other.clone(),
        };
        out.insert(k.clone(), v);
    }
    Value::Object(out)
}

/// 由一次调用的输入输出生成日志记录。
/// - `media_key`：参数里的 media_key，否则取返回 JSON 的 media_key；
/// - `bytes`：所有 text content 的字节数 + 图片 base64 长度；
/// - `response_mode`：返回 JSON 的 response_mode 字段（仅 analyze_media 带）；
/// - `ok`：返回 JSON 没有 `"error": true` 且不是协议错误。
pub fn record(
    tool: &str,
    args: Option<&Map<String, Value>>,
    texts: &[&str],
    image_bytes: usize,
    protocol_ok: bool,
) -> ToolCallRecord {
    let first: Option<Value> = texts.first().and_then(|t| serde_json::from_str(t).ok());
    let field = |name: &str| -> Option<String> {
        first
            .as_ref()
            .and_then(|v| v.get(name))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let media_key = args
        .and_then(|a| a.get("media_key"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| field("media_key"));
    let is_err = first
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    ToolCallRecord {
        ts: now_rfc3339(),
        tool: tool.to_string(),
        media_key,
        args: summarize_args(args),
        bytes: texts.iter().map(|t| t.len()).sum::<usize>() + image_bytes,
        response_mode: field("response_mode"),
        ok: protocol_ok && !is_err,
    }
}

/// UTC RFC3339（秒精度），不引入 chrono。
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant 的 civil_from_days。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_and_record() {
        let args = json!({
            "media_key": "abc",
            "start": 1.5,
            "end": 30,
            "cursor": "opaque-token",
            "times": [1.0, 2.0, 3.0],
            "source": "x".repeat(200),
        });
        let rec = record(
            "read_transcript",
            args.as_object(),
            &[r#"{"segments":[]}"#],
            0,
            true,
        );
        assert_eq!(rec.media_key.as_deref(), Some("abc"));
        assert_eq!(rec.args["start"], 1.5);
        assert_eq!(rec.args["cursor"], true);
        assert_eq!(rec.args["times"], json!({ "len": 3 }));
        assert!(rec.args.get("media_key").is_none());
        assert_eq!(
            rec.args["source"].as_str().unwrap().chars().count(),
            ARG_STR_MAX + 1
        );
        assert_eq!(rec.bytes, 15);
        assert!(rec.ok && rec.response_mode.is_none());

        let out = r#"{"media_key":"k2","response_mode":"index","sections":[]}"#;
        let rec = record("analyze_media", None, &[out], 10, true);
        assert_eq!(rec.media_key.as_deref(), Some("k2"));
        assert_eq!(rec.response_mode.as_deref(), Some("index"));
        assert_eq!(rec.bytes, out.len() + 10);

        let rec = record("get_frame", None, &[r#"{"error":true}"#], 0, true);
        assert!(!rec.ok);
        assert!(rec.ts.ends_with('Z') && rec.ts.len() == 20);
    }

    #[test]
    fn writes_jsonl_line() {
        let tmp = tempfile::TempDir::new().unwrap();
        let log = CallLog::in_logs_dir(&tmp.path().join("logs"));
        log.write(&record("x", None, &["{}"], 0, true));
        log.write(&record("y", None, &["{}"], 0, true));
        let s = std::fs::read_to_string(log.path().unwrap()).unwrap();
        let lines: Vec<Value> = s
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["tool"], "y");
    }
}
