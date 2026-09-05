//! 工具下载与安装：流式下载、SHA256 校验、zip 解包、原子落地。

use std::path::{Path, PathBuf};

use reqwest::Client;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tracing::info;

use crate::error::{AvrError, ErrorCode};
use crate::tools::manifest::{ArchiveLayout, ToolManifest};

/// 下载并安装工具到数据根。
pub async fn download_and_install(
    manifest: &ToolManifest,
    data_root: &Path,
) -> Result<PathBuf, AvrError> {
    let install_dir = data_root.join(&manifest.install_dir);
    std::fs::create_dir_all(&install_dir).map_err(io_err)?;

    let tmp_dir = data_root.join("logs").join("install-tmp");
    std::fs::create_dir_all(&tmp_dir).map_err(io_err)?;

    let archive_path = tmp_dir.join(format!("{}.part", manifest.id));
    download_file(&manifest.url, &archive_path).await?;
    verify_sha256(&archive_path, &manifest.sha256)?;

    let result = match &manifest.archive_layout {
        ArchiveLayout::SingleExe { path } => {
            let dest = install_dir.join(path);
            atomic_copy(&archive_path, &dest)?;
            dest
        }
        ArchiveLayout::ZipExtract { entries } => {
            extract_zip_entries(&archive_path, &install_dir, entries)?;
            install_dir.join(&entries[0])
        }
    };

    std::fs::remove_file(&archive_path).ok();
    info!("已安装 {} -> {}", manifest.id, result.display());
    Ok(result)
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
    let data = std::fs::read(path).map_err(io_err)?;
    let hash = Sha256::digest(&data);
    let got = format!("{:x}", hash);
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
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        AvrError::new(ErrorCode::ToolNotReady, e.to_string(), "压缩包损坏。")
    })?;

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
            let part = out_path.with_extension("part");
            let mut out = std::fs::File::create(&part).map_err(io_err)?;
            std::io::copy(&mut entry, &mut out).map_err(io_err)?;
            atomic_rename(&part, &out_path)?;
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

fn atomic_copy(from: &Path, to: &Path) -> Result<(), AvrError> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    let part = to.with_extension("part");
    std::fs::copy(from, &part).map_err(io_err)?;
    atomic_rename(&part, to)?;
    Ok(())
}

fn atomic_rename(from: &Path, to: &Path) -> Result<(), AvrError> {
    if to.exists() {
        std::fs::remove_file(to).map_err(io_err)?;
    }
    std::fs::rename(from, to).map_err(io_err)?;
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
