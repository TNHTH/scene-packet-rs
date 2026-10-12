//! 共用文件 IO；缓存、模型身份和导出共用完整流式哈希。
use crate::error::{process_failed, AvrError};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn sha256(path: &Path) -> Result<String, AvrError> {
    let mut file = std::fs::File::open(path).map_err(|e| process_failed(&e.to_string()))?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n =
            std::io::Read::read(&mut file, &mut buf).map_err(|e| process_failed(&e.to_string()))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn atomic_write(path: &Path, content: &[u8]) -> Result<(), AvrError> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| AvrError::io(e, "检查目录权限。"))?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| AvrError::io(e, "检查目录权限。"))?;
    tmp.write_all(content)
        .and_then(|_| tmp.as_file().sync_all())
        .map_err(|e| AvrError::io(e, "检查磁盘空间。"))?;
    tmp.persist(path)
        .map_err(|e| AvrError::io(e.error, "检查目标文件权限。"))?;
    Ok(())
}
