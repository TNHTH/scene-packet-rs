//! 大文件内容指纹：大小 + 头尾各 1MB 的 xxh3。
//!
//! 用于 ASR 模型（可能几百 MB）与工具二进制的缓存失效判断：内容变了指纹就变；
//! 只 touch、重装同一文件、复制到别的路径，指纹不变。结果按 (路径, 大小, mtime)
//! 在进程内缓存，避免每次分析都重新读文件。

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

use parking_lot::Mutex;
use xxhash_rust::xxh3::Xxh3;

const CHUNK: u64 = 1024 * 1024;

type Key = (PathBuf, u64, Option<SystemTime>);
static CACHE: LazyLock<Mutex<HashMap<Key, String>>> = LazyLock::new(Default::default);

/// 文件内容指纹，形如 `"<size>-<xxh3 hex>"`；文件不存在或不可读时返回 None。
/// 指纹不包含路径。
pub fn content_fingerprint(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(hit) = CACHE.lock().get(&key) {
        return Some(hit.clone());
    }
    let fp = compute(path, meta.len()).ok()?;
    CACHE.lock().insert(key, fp.clone());
    Some(fp)
}

fn compute(path: &Path, len: u64) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Xxh3::new();
    let mut buf = Vec::with_capacity(CHUNK as usize);
    (&mut f).take(CHUNK).read_to_end(&mut buf)?;
    hasher.update(&buf);
    if len > CHUNK {
        // 尾部 1MB（与头部重叠也无妨，长度已计入指纹）。
        let tail_start = len.saturating_sub(CHUNK).max(CHUNK.min(len));
        f.seek(SeekFrom::Start(tail_start))?;
        buf.clear();
        f.take(CHUNK).read_to_end(&mut buf)?;
        hasher.update(&buf);
    }
    Ok(format!("{len}-{:016x}", hasher.digest()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_content_same_fingerprint_across_paths_and_touch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let a = tmp.path().join("a.bin");
        let data: Vec<u8> = (0..3 * CHUNK as usize).map(|i| (i % 251) as u8).collect();
        std::fs::write(&a, &data).unwrap();
        let fa = content_fingerprint(&a).unwrap();

        let b = tmp.path().join("b.bin");
        std::fs::copy(&a, &b).unwrap();
        assert_eq!(content_fingerprint(&b).unwrap(), fa, "复制到新路径不变");

        let f = File::options().write(true).open(&a).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        assert_eq!(content_fingerprint(&a).unwrap(), fa, "只 touch 不变");

        let mut changed = data.clone();
        *changed.last_mut().unwrap() ^= 0xff; // 尾部变化
        std::fs::write(&a, &changed).unwrap();
        assert_ne!(content_fingerprint(&a).unwrap(), fa, "尾部内容变化");
        assert!(content_fingerprint(&tmp.path().join("none")).is_none());
    }
}
