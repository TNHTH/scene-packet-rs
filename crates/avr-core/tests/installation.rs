use std::io::{Read, Write};
fn install(payload: Vec<u8>, missing: bool) {
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("bin/test");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("tool"), b"old").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let (mut s, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && start.elapsed().as_secs() < 10 =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(e) => panic!("fixture accept: {e}"),
            }
        };
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        let _ = s.read(&mut buf);
        write!(
            s,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        )
        .unwrap();
        s.write_all(&payload).unwrap();
    });
    let m = avr_core::tools::manifest::ToolManifest {
        id: "test".into(),
        version: "1".into(),
        url: format!("http://{addr}"),
        sha256: String::new(),
        license: String::new(),
        license_url: String::new(),
        source_url: String::new(),
        install_dir: "bin/test".into(),
        archive_layout: if missing {
            avr_core::tools::manifest::ArchiveLayout::ZipExtract {
                entries: vec!["missing".into()],
            }
        } else {
            avr_core::tools::manifest::ArchiveLayout::SingleExe {
                path: "tool".into(),
            }
        },
        depends: vec![],
        optional: vec![],
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let r = rt.block_on(avr_core::tools::install::download_and_install(&m, d.path()));
    server.join().unwrap();
    if missing {
        assert!(r.is_err());
        assert_eq!(std::fs::read(target.join("tool")).unwrap(), b"old");
    } else {
        assert!(r.is_ok());
        assert_eq!(std::fs::read(target.join("tool")).unwrap(), b"new");
    }
}
#[test]
fn successful_install_replaces_old() {
    install(b"new".to_vec(), false);
}
#[test]
fn invalid_zip_keeps_old() {
    install(b"invalid zip".to_vec(), true);
}
#[test]
fn held_install_lock_must_prevent_download() {
    let d = tempfile::tempdir().unwrap();
    let bin = d.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(bin.join(".test.install.lock"))
        .unwrap();
    assert!(fs4::fs_std::FileExt::try_lock_exclusive(&lock).unwrap());
    let m = avr_core::tools::manifest::ToolManifest {
        id: "test".into(),
        version: "1".into(),
        url: "http://127.0.0.1:1".into(),
        sha256: String::new(),
        license: String::new(),
        license_url: String::new(),
        source_url: String::new(),
        install_dir: "bin/test".into(),
        archive_layout: avr_core::tools::manifest::ArchiveLayout::SingleExe {
            path: "tool".into(),
        },
        depends: vec![],
        optional: vec![],
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let e = rt
        .block_on(avr_core::tools::install::download_and_install(&m, d.path()))
        .unwrap_err();
    assert_eq!(e.code, avr_core::ErrorCode::CacheError);
}
