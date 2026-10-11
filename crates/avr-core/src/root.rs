//! 数据根定位与事务化迁移。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::error::{AvrError, ErrorCode};
use crate::paths::{default_data_root, root_pointer_path, PORTABLE_MARKER};

pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvrRootMeta {
    pub schema_version: u32,
    pub created_by: String,
}

impl Default for AvrRootMeta {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            created_by: format!("avr {}", crate::VERSION),
        }
    }
}

/// 已解析的数据根目录。
#[derive(Debug, Clone)]
pub struct DataRoot {
    pub path: PathBuf,
}

impl DataRoot {
    pub fn config_path(&self) -> PathBuf {
        self.path.join("config.toml")
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.path.join("bin")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.path.join("models")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.path.join("cache")
    }

    pub fn exports_dir(&self) -> PathBuf {
        self.path.join("exports")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.path.join("logs")
    }

    pub fn ensure_layout(&self) -> Result<(), AvrError> {
        for sub in ["bin", "models", "cache", "exports", "logs"] {
            fs::create_dir_all(self.path.join(sub)).map_err(io_err)?;
        }
        let meta_path = self.path.join(".avr-root");
        if !meta_path.exists() {
            let meta = AvrRootMeta::default();
            fs::write(
                &meta_path,
                serde_json::to_string_pretty(&meta).map_err(|e| {
                    AvrError::new(ErrorCode::IoError, "序列化失败", "检查磁盘权限。").with_source(e)
                })?,
            )
            .map_err(io_err)?;
        }
        Ok(())
    }
}

fn io_err(e: std::io::Error) -> AvrError {
    AvrError::io(e, "检查磁盘权限与路径。")
}

/// 定位数据根：AVR_DATA_ROOT > avr.portable > %APPDATA%/root.txt
pub fn resolve_data_root(exe_dir: Option<&Path>) -> Result<DataRoot, AvrError> {
    if let Ok(env) = std::env::var("AVR_DATA_ROOT") {
        let p = PathBuf::from(env);
        info!("数据根来自 AVR_DATA_ROOT: {}", p.display());
        return Ok(DataRoot { path: p });
    }

    if let Some(dir) = exe_dir {
        let portable = dir.join(PORTABLE_MARKER);
        if portable.exists() {
            info!("便携模式: {}", dir.display());
            return Ok(DataRoot {
                path: dir.to_path_buf(),
            });
        }
    }

    let pointer = root_pointer_path();
    if pointer.exists() {
        let content = fs::read_to_string(&pointer).map_err(io_err)?;
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            info!("数据根来自指针: {}", p.display());
            return Ok(DataRoot { path: p });
        }
    }

    let fallback = default_data_root();
    warn!("使用默认数据根: {}", fallback.display());
    Ok(DataRoot { path: fallback })
}

/// 是否需要首次启动向导（无 root.txt 指针且非便携/环境变量模式）。
pub fn needs_first_run_setup(exe_dir: Option<&Path>) -> bool {
    if std::env::var("AVR_DATA_ROOT").is_ok() {
        return false;
    }
    if let Some(dir) = exe_dir {
        if dir.join(PORTABLE_MARKER).exists() {
            return false;
        }
    }
    !root_pointer_path().exists()
}

/// 写入 root.txt 指针（事务：先 .tmp 再 rename）。
pub fn set_root_pointer(target: &Path) -> Result<(), AvrError> {
    let pointer = root_pointer_path();
    if let Some(parent) = pointer.parent() {
        fs::create_dir_all(parent).map_err(io_err)?;
    }
    let tmp = pointer.with_extension("txt.tmp");
    fs::write(&tmp, target.display().to_string()).map_err(io_err)?;
    fs::rename(&tmp, &pointer).map_err(io_err)?;
    Ok(())
}

/// 事务化迁移数据根。
pub fn migrate_data_root(old: &Path, new: &Path) -> Result<(), AvrError> {
    if old == new {
        return Ok(());
    }
    fs::create_dir_all(new).map_err(io_err)?;

    let lock = old.join(".migrate.lock");
    if lock.exists() {
        return Err(AvrError::new(
            ErrorCode::IoError,
            "检测到未完成的迁移锁。",
            "手动删除 .migrate.lock 后重试，或联系支持。",
        ));
    }
    fs::write(&lock, "1").map_err(io_err)?;

    copy_dir_recursive(old, new)?;

    let old_count = count_files(old)?;
    let new_count = count_files(new)?;
    if new_count < old_count {
        fs::remove_file(&lock).ok();
        return Err(AvrError::new(
            ErrorCode::IoError,
            format!("迁移校验失败: 旧={old_count} 新={new_count}"),
            "检查目标磁盘空间后重试。",
        ));
    }

    set_root_pointer(new)?;
    fs::remove_file(&lock).ok();
    info!("迁移完成: {} -> {}", old.display(), new.display());
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), AvrError> {
    fs::create_dir_all(dst).map_err(io_err)?;
    for entry in fs::read_dir(src).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        let name = entry.file_name();
        if name == ".migrate.lock" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(name);
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(io_err)?;
        }
    }
    Ok(())
}

fn count_files(dir: &Path) -> Result<usize, AvrError> {
    let mut n = 0;
    if !dir.exists() {
        return Ok(0);
    }
    for entry in fs::read_dir(dir).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        if entry.path().is_dir() {
            n += count_files(&entry.path())?;
        } else {
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tempfile::TempDir;

    #[test]
    fn resolve_from_env() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_string_lossy().to_string();
        env::set_var("AVR_DATA_ROOT", &path);
        let root = resolve_data_root(None).unwrap();
        assert_eq!(root.path, PathBuf::from(path));
        env::remove_var("AVR_DATA_ROOT");
    }

    #[test]
    fn resolve_portable() {
        env::remove_var("AVR_DATA_ROOT");
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(PORTABLE_MARKER), "").unwrap();
        let root = resolve_data_root(Some(tmp.path())).unwrap();
        assert_eq!(root.path, tmp.path());
    }
}
