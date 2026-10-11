//! MCP 工具调用日志：每次调用一行 JSON，总是写 stderr；`[mcp] call_log = true` 时
//! 另追加到 `<数据根>/logs/mcp-calls.jsonl`（超过 `call_log_max_bytes` 轮转为 `.1`）。
//! 不写 stdout（stdio 传输的协议通道）。URL 只记域名、本地路径只记扩展名，在记录生成时完成，两路输出一致。

use std::io::Write;
use std::path::{Path, PathBuf};

use avr_core::config::McpConfig;
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

/// 文件日志名。
pub const CALL_LOG_FILE: &str = "mcp-calls.jsonl";

/// 调用日志写入器。`path` 为 None 时只写 stderr。
#[derive(Debug, Clone, Default)]
pub struct CallLog {
    path: Option<PathBuf>,
    max_bytes: u64,
}

impl CallLog {
    /// `path` 为 None 只写 stderr；文件超过 `max_bytes` 时轮转。
    pub fn new(path: Option<PathBuf>, max_bytes: u64) -> Self {
        Self { path, max_bytes }
    }

    /// 按配置构造：`call_log = true` 才写 `<logs_dir>/mcp-calls.jsonl`。
    pub fn from_config(logs_dir: &Path, cfg: &McpConfig) -> Self {
        let path = cfg.call_log.then(|| logs_dir.join(CALL_LOG_FILE));
        Self::new(path, cfg.call_log_max_bytes)
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
            self.rotate_if_needed(path, line.len() as u64 + 1);
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

impl CallLog {
    /// 追加后会超过上限就把当前文件改名为 `.1`（覆盖旧 `.1`），随后新建。
    fn rotate_if_needed(&self, path: &Path, incoming: u64) {
        let Ok(meta) = std::fs::metadata(path) else {
            return;
        };
        if meta.len() == 0 || meta.len() + incoming <= self.max_bytes {
            return;
        }
        let mut rotated = path.as_os_str().to_owned();
        rotated.push(".1");
        let rotated = PathBuf::from(rotated);
        // Windows 上 rename 不覆盖已有文件。
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(path, &rotated);
    }
}

/// URL 只留域名（`url:host`），本地路径只留扩展名（`path:.mp4`，无扩展名为 `path:`）。
pub fn redact(s: &str) -> String {
    if let Some((_, rest)) = s.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host_port = authority.rsplit('@').next().unwrap_or("");
        let host = if host_port.starts_with('[') {
            host_port
                .split(']')
                .next()
                .map(|h| format!("{h}]"))
                .unwrap_or_default()
        } else {
            host_port.split(':').next().unwrap_or("").to_string()
        };
        format!("url:{host}")
    } else {
        let name = s.rsplit(['/', '\\']).next().unwrap_or("");
        let ext = name
            .rsplit_once('.')
            .filter(|(stem, _)| !stem.is_empty())
            .map(|(_, e)| format!(".{e}"))
            .unwrap_or_default();
        format!("path:{ext}")
    }
}

/// 需要脱敏的参数：`source` 及名字以 path/url 结尾的键，或值看起来像 URL/路径。
fn is_sensitive(key: &str, val: &str) -> bool {
    key == "source"
        || key.ends_with("path")
        || key.ends_with("url")
        || val.contains("://")
        || val.contains('/')
        || val.contains('\\')
}

/// 字符串参数在日志里最多保留的字符数。
const ARG_STR_MAX: usize = 120;

/// 参数摘要：去掉 media_key（单独成字段）；URL/路径脱敏（见 [`redact`]）；长字符串截断，数组只记长度（如 `times` → `{"len":3}`），
/// cursor 只记是否存在。
pub fn summarize_args(args: Option<&Map<String, Value>>) -> Value {
    let mut out = Map::new();
    for (k, v) in args.into_iter().flatten() {
        if k == "media_key" {
            continue;
        }
        let v = match v {
            Value::String(_) if k == "cursor" => Value::Bool(true),
            Value::String(s) if is_sensitive(k, s) => Value::String(redact(s)),
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
/// - `media_key`：参数里的 media_key，否则取返回 JSON 的 media_key；原样记录（派生 ID）；
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
        assert_eq!(rec.args["source"], "path:");
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

    fn cfg(call_log: bool, max: u64) -> McpConfig {
        McpConfig {
            call_log,
            call_log_max_bytes: max,
            ..Default::default()
        }
    }

    #[test]
    fn redacts_urls_and_paths() {
        let url = "https://user:pw@www.bilibili.com:443/video/BV1xx?p=2&token=SECRET";
        assert_eq!(redact(url), "url:www.bilibili.com");
        assert_eq!(redact("https://v.douyin.com/abc/"), "url:v.douyin.com");
        assert_eq!(redact(r"C:\Users\张三\Videos\私密.mp4"), "path:.mp4");
        assert_eq!(redact("/home/alice/clip.MKV"), "path:.MKV");
        assert_eq!(redact("/home/a/.bashrc"), "path:");

        let args = json!({
            "source": "/home/alice/secret.mp4",
            "video_path": r"D:\x\y.wav",
            "page_url": "https://www.bilibili.com/video/BV1xx?token=SECRET",
            "response_mode": "index",
            "media_key": "abc123",
        });
        let rec = record("analyze_media", args.as_object(), &["{}"], 0, true);
        let line = serde_json::to_string(&rec).unwrap();
        for leak in ["alice", "secret", "y.wav", "BV1xx", "SECRET"] {
            assert!(!line.contains(leak), "{leak} 泄漏: {line}");
        }
        assert_eq!(rec.args["source"], "path:.mp4");
        assert_eq!(rec.args["video_path"], "path:.wav");
        assert_eq!(rec.args["page_url"], "url:www.bilibili.com");
        assert_eq!(rec.args["response_mode"], "index");
        assert_eq!(rec.media_key.as_deref(), Some("abc123"));
    }

    #[test]
    fn file_log_off_by_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let logs = tmp.path().join("logs");
        let log = CallLog::from_config(&logs, &McpConfig::default());
        assert!(log.path().is_none());
        log.write(&record("x", None, &["{}"], 0, true));
        assert!(!logs.join(CALL_LOG_FILE).exists());
    }

    #[test]
    fn rotates_over_limit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let log = CallLog::from_config(tmp.path(), &cfg(true, 400));
        let path = log.path().unwrap().to_path_buf();
        let rotated = tmp.path().join(format!("{CALL_LOG_FILE}.1"));
        for i in 0..20 {
            log.write(&record(&format!("tool{i}"), None, &["{}"], 0, true));
            assert!(std::fs::metadata(&path).unwrap().len() <= 400);
        }
        assert!(rotated.exists());
        assert!(std::fs::metadata(&rotated).unwrap().len() <= 400);
        let last = std::fs::read_to_string(&path).unwrap();
        assert!(last.lines().last().unwrap().contains("\"tool19\""));
        // 轮转只保留一个 .1。
        assert!(!tmp.path().join(format!("{CALL_LOG_FILE}.2")).exists());
    }

    #[test]
    fn writes_jsonl_line() {
        let tmp = tempfile::TempDir::new().unwrap();
        let log = CallLog::from_config(&tmp.path().join("logs"), &cfg(true, 5 << 20));
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
