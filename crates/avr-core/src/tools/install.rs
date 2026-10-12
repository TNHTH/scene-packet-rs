//! 工具下载与安装：流式下载、SHA256 校验、zip 解包、原子落地。

use std::path::{Path, PathBuf};

use reqwest::Client;
use tokio::io::AsyncWriteExt;
use tracing::info;

use crate::error::{AvrError, ErrorCode};
use crate::tools::manifest::{ArchiveLayout, ToolManifest};

/// 下载并安装工具到数据根。
pub async fn download_and_install(
    manifest: &ToolManifest,
    data_root: &Path,
) -> Result<PathBuf, AvrError> {
    validate_relative(&manifest.install_dir)?;
    let install_dir = data_root.join(&manifest.install_dir);
    let parent = install_dir
        .parent()
        .ok_or_else(|| install_error("安装目录无父目录"))?;
    std::fs::create_dir_all(parent).map_err(io_err)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(format!(
            ".{}.install.lock",
            install_dir.file_name().unwrap().to_string_lossy()
        )))
        .map_err(io_err)?;
    fs4::fs_std::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| install_error("此工具正在安装，请稍后重试"))?;
    let work = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(parent)
        .map_err(io_err)?;
    let archive_path = work.path().join("download");
    download_file(&manifest.url, &archive_path).await?;
    verify_sha256(&archive_path, &manifest.sha256)?;
    let stage = work.path().join("stage");
    std::fs::create_dir(&stage).map_err(io_err)?;
    if install_dir.exists() {
        copy_existing(&install_dir, &stage)?;
    }
    let main = stage_archive(manifest, &archive_path, &stage)?;
    let backup = work.path().join("backup");
    let had_previous = install_dir.exists();
    if had_previous {
        std::fs::rename(&install_dir, &backup).map_err(io_err)?;
    }
    if let Err(error) = std::fs::rename(&stage, &install_dir) {
        if had_previous {
            if let Err(rollback) = std::fs::rename(&backup, &install_dir) {
                let retained = work.keep();
                return Err(install_error(&format!(
                    "安装发布失败: {error}；恢复失败: {rollback}；旧安装保留在 {}",
                    retained.join("backup").display()
                )));
            }
        }
        return Err(io_err(error));
    }
    let result = install_dir.join(main);
    info!("已安装 {} -> {}", manifest.id, result.display());
    Ok(result)
}

fn install_error(message: &str) -> AvrError {
    AvrError::new(
        ErrorCode::ToolNotReady,
        message,
        "已有安装保持不变；检查清单或重试。",
    )
}
fn validate_relative(value: &str) -> Result<(), AvrError> {
    if value.is_empty()
        || value.contains(['\\', ':'])
        || Path::new(value)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(install_error("安装清单路径必须为相对路径"));
    }
    Ok(())
}
fn copy_existing(from: &Path, to: &Path) -> Result<(), AvrError> {
    for entry in std::fs::read_dir(from).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        let kind = entry.file_type().map_err(io_err)?;
        let dest = to.join(entry.file_name());
        if kind.is_symlink() {
            return Err(install_error("工具目录含符号链接，请手动处理后安装"));
        }
        if kind.is_dir() {
            std::fs::create_dir(&dest).map_err(io_err)?;
            copy_existing(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest).map_err(io_err)?;
        }
    }
    Ok(())
}
fn stage_archive(
    manifest: &ToolManifest,
    archive: &Path,
    stage: &Path,
) -> Result<PathBuf, AvrError> {
    match &manifest.archive_layout {
        ArchiveLayout::SingleExe { path } => {
            validate_relative(path)?;
            let destination = stage.join(path);
            std::fs::create_dir_all(destination.parent().unwrap()).map_err(io_err)?;
            std::fs::copy(archive, destination).map_err(io_err)?;
            Ok(path.into())
        }
        ArchiveLayout::ZipExtract { entries } => {
            let first = entries
                .first()
                .ok_or_else(|| install_error("安装清单未声明输出"))?;
            for entry in entries {
                validate_relative(entry)?;
            }
            extract_zip_entries(archive, stage, entries)?;
            // Whisper 随包 DLL 也是运行时的一部分，不能只抽取两项。
            if manifest.id.starts_with("whisper") {
                extract_runtime_files(archive, stage)?;
            }
            Ok(first.into())
        }
    }
}

fn extract_runtime_files(path: &Path, dest: &Path) -> Result<(), AvrError> {
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(path).map_err(io_err)?).map_err(zip_err)?;
    let mut names = std::collections::HashSet::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_err)?;
        if entry.is_dir() {
            continue;
        }
        let normalized = entry.name().replace('\\', "/");
        let path = Path::new(&normalized);
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "exe" | "dll") {
            continue;
        }
        let name = path
            .file_name()
            .ok_or_else(|| install_error("无效运行时文件名"))?;
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(install_error("压缩包存在重名运行时文件"));
        }
        let mut file = std::fs::File::create(dest.join(name)).map_err(io_err)?;
        std::io::copy(&mut entry, &mut file).map_err(io_err)?;
    }
    Ok(())
}

async fn download_file(url: &str, dest: &Path) -> Result<(), AvrError> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(net_err)?;

    let resp = client.get(url).send().await.map_err(net_err)?;
    if !resp.status().is_success() {
        return Err(AvrError::new(
            ErrorCode::ToolNotReady,
            format!("下载失败 HTTP {}", resp.status()),
            "检查网络或稍后重试。",
        ));
    }

    let mut file = tokio::fs::File::create(dest).await.map_err(io_err)?;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(net_err)?;
        file.write_all(&chunk).await.map_err(io_err)?;
    }
    file.flush().await.map_err(io_err)?;
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), AvrError> {
    if expected.is_empty() {
        return Ok(());
    }
    let got = crate::storage::sha256(path)?;
    if got != expected.to_lowercase() {
        return Err(AvrError::new(
            ErrorCode::ToolNotReady,
            format!("SHA256 不匹配: 期望 {expected}, 实际 {got}"),
            "重新下载或手动安装。",
        ));
    }
    Ok(())
}

fn extract_zip_entries(
    zip_path: &Path,
    dest_dir: &Path,
    entry_names: &[String],
) -> Result<(), AvrError> {
    let file = std::fs::File::open(zip_path).map_err(io_err)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AvrError::new(ErrorCode::ToolNotReady, e.to_string(), "压缩包损坏。"))?;

    for want in entry_names {
        let want_base = Path::new(want)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| want.clone());

        let mut found = false;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(zip_err)?;
            let name = entry.name().replace('\\', "/");
            let base = Path::new(&name)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if base != want_base && !name.ends_with(want.as_str()) {
                continue;
            }
            let out_path = dest_dir.join(want);
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut out = std::fs::File::create(&out_path).map_err(io_err)?;
            std::io::copy(&mut entry, &mut out).map_err(io_err)?;

            found = true;
            break;
        }
        if !found {
            return Err(AvrError::new(
                ErrorCode::ToolNotReady,
                format!("压缩包中未找到 {want}"),
                "尝试手动解压到数据根 bin/ 目录。",
            ));
        }
    }
    Ok(())
}

fn io_err(e: std::io::Error) -> AvrError {
    AvrError::new(ErrorCode::IoError, e.to_string(), "检查磁盘权限。")
}

fn net_err(e: reqwest::Error) -> AvrError {
    AvrError::new(ErrorCode::NetworkError, e.to_string(), "检查网络连接。")
}

fn zip_err(e: zip::result::ZipError) -> AvrError {
    AvrError::new(ErrorCode::ToolNotReady, e.to_string(), "压缩包损坏。")
}
