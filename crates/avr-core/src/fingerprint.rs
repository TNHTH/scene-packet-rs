//! 完整文件内容指纹。按内容确定缓存身份，不依赖可保留的 mtime 或头尾采样。
use std::path::Path;

/// 文件不存在或不可读时返回 None。路径和时间戳不参与内容身份。
pub fn content_fingerprint(path: &Path) -> Option<String> {
    crate::storage::sha256(path)
        .ok()
        .map(|hash| format!("sha256:{hash}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::time::SystemTime;
    const CHUNK: u64 = 1024 * 1024;

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
