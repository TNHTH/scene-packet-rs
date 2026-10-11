//! Tauri 命令的可序列化错误包装。核心库的 `AvrError` 不依赖 serde，
//! 在这一层转成 `{ code, message, display }` 交给前端。

use avr_core::error::AvrError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    /// 稳定错误码，如 `IO_ERROR`。
    pub code: String,
    /// `AvrError::to_string()`：错误码 + 简要说明。
    pub message: String,
    /// 面向用户的完整文本（含原因链与下一步建议）。
    pub display: String,
}

impl CommandError {
    /// 非核心库错误（例如前端传入了未知参数）。
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: "INVALID_ARGUMENT".into(),
            display: message.clone(),
            message,
        }
    }
}

impl From<AvrError> for CommandError {
    fn from(e: AvrError) -> Self {
        Self {
            code: e.code.as_str().into(),
            message: e.to_string(),
            display: e.format_user(),
        }
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_code_and_message() {
        let io = std::io::Error::other("磁盘已满");
        let e: CommandError = AvrError::io(io, "清理磁盘").into();
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "IO_ERROR");
        assert_eq!(v["message"], "[IO_ERROR] 文件读写失败");
        assert!(v["display"].as_str().unwrap().contains("磁盘已满"));
    }
}
