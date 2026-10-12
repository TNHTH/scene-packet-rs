//! 数据根定位、校验复制与指针切换。

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
    crate::storage::atomic_write(&pointer, target.display().to_string().as_bytes())?;
    Ok(())
}

/// 复制并逐文件校验后切换指针；保留原目录。迁移前应停止使用该数据根的作业。
pub fn migrate_data_root(old: &Path, new: &Path) -> Result<(), AvrError> {
    let old = fs::canonicalize(old).map_err(io_err)?;
    let parent = new
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(io_err)?;
    let parent = fs::canonicalize(parent).map_err(io_err)?;
    let name = new
        .file_name()
        .ok_or_else(|| migration_error("迁移目标无效"))?;
    let target = parent.join(name);
    if fs::canonicalize(&target).ok().as_ref() == Some(&old) {
        return Ok(());
    }
    if target.starts_with(&old) || old.starts_with(&target) {
        return Err(migration_error("源目录和目标目录不能互相包含"));
    }
    if target.exists()
        && (!target.is_dir() || fs::read_dir(&target).map_err(io_err)?.next().is_some())
    {
        return Err(migration_error("迁移目标必须不存在或为空目录"));
    }
    if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(migration_error("迁移目标不能是符号链接"));
    }
    let _lock = crate::jobs::FileLock::try_acquire(&old.join(".migrate.lock"))?;
    let staged = tempfile::tempdir_in(&parent).map_err(io_err)?;
    copy_dir_verified(&old, staged.path())?;
    if target.exists() {
        fs::remove_dir(&target).map_err(io_err)?;
    }
    fs::rename(staged.path(), &target).map_err(io_err)?;
    set_root_pointer(&target)?;
    info!("迁移完成: {} -> {}", old.display(), target.display());
    Ok(())
}

fn migration_error(message: &str) -> AvrError {
    AvrError::new(
        ErrorCode::IoError,
        message,
        "使用独立的空目标目录，停止作业后重试；原目录会保留。",
    )
}

fn copy_dir_verified(src: &Path, dst: &Path) -> Result<(), AvrError> {
    fs::create_dir_all(dst).map_err(io_err)?;
    for entry in fs::read_dir(src).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        let name = entry.file_name();
        if name == ".migrate.lock" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(name);
        let kind = entry.file_type().map_err(io_err)?;
        if kind.is_dir() {
            copy_dir_verified(&from, &to)?;
        } else if kind.is_file() {
            fs::copy(&from, &to).map_err(io_err)?;
            if crate::storage::sha256(&from)? != crate::storage::sha256(&to)? {
                return Err(migration_error("迁移文件校验失败"));
            }
        } else {
            return Err(migration_error("数据根包含符号链接或非普通文件"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tempfile::TempDir;

    #[test]
    fn migration_rejects_nested_or_nonempty_destination() {
        let tmp = TempDir::new().unwrap();
        let old = tmp.path().join("old");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("source"), b"old").unwrap();
        assert!(migrate_data_root(&old, &old.join("nested")).is_err());
        let dest = tmp.path().join("existing");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("user"), b"preserve").unwrap();
        assert!(migrate_data_root(&old, &dest).is_err());
        assert_eq!(fs::read(dest.join("user")).unwrap(), b"preserve");
    }

    #[test]
    fn migration_copy_skips_lock_and_verifies_content() {
        let tmp = TempDir::new().unwrap();
        let old = tmp.path().join("old");
        let dest = tmp.path().join("copy");
        fs::create_dir_all(old.join("models")).unwrap();
        fs::write(old.join(".migrate.lock"), b"stale").unwrap();
        fs::write(old.join("models/model"), b"model").unwrap();
        copy_dir_verified(&old, &dest).unwrap();
        assert!(!dest.join(".migrate.lock").exists());
        assert_eq!(fs::read(dest.join("models/model")).unwrap(), b"model");
    }

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
