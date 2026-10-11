//! 路径与 URI 工具。

use std::path::{Path, PathBuf};

/// 转为 file URI（Windows 原生路径）。
pub fn to_file_uri(path: &Path) -> String {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    url::Url::from_file_path(&abs)
        .map(|url| url.into())
        .unwrap_or_else(|_| String::new())
}

/// 默认数据根（未配置时）。
pub fn default_data_root() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("AIVideoReader")
    } else if let Ok(home) = std::env::var("USERPROFILE") {
        PathBuf::from(home)
            .join("AppData")
            .join("Roaming")
            .join("AIVideoReader")
    } else if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(data).join("ScenePacket")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/ScenePacket")
    } else {
        PathBuf::from(".avr-data")
    }
}

pub fn root_pointer_path() -> PathBuf {
    default_data_root().join("root.txt")
}

pub const PORTABLE_MARKER: &str = "avr.portable";
