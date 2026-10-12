use avr_core::{
    fingerprint::content_fingerprint,
    media::{media_key, MediaSource},
    timeline::{parse_srt, parse_vtt, segments_to_srt},
};
use std::io::{Seek, SeekFrom, Write};

#[test]
fn model_middle_change_with_preserved_mtime_invalidates_cache() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("model");
    std::fs::write(&path, vec![0; 4 * 1024 * 1024]).unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let mtime = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100);
    file.set_modified(mtime).unwrap();
    let before = content_fingerprint(&path).unwrap();
    file.seek(SeekFrom::Start(2 * 1024 * 1024)).unwrap();
    file.write_all(b"changed").unwrap();
    file.set_modified(mtime).unwrap();
    assert_ne!(before, content_fingerprint(&path).unwrap());
}

#[test]
fn media_tail_change_invalidates_cache() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("clip");
    let mut data = vec![0; 5 * 1024 * 1024];
    std::fs::write(&path, &data).unwrap();
    let source = MediaSource::LocalFile { path: path.clone() };
    let before = media_key(&source, Some(10.0)).unwrap();
    *data.last_mut().unwrap() = 1;
    std::fs::write(&path, data).unwrap();
    assert_ne!(before, media_key(&source, Some(10.0)).unwrap());
}

#[test]
fn windows_srt_and_subsecond_rounding() {
    let text = "1\r\n00:00:00,000 --> 00:00:01,000\r\n甲\r\n\r\n2\r\n00:00:02,000 --> 00:00:03,000\r\n乙\r\n";
    let mut t = parse_srt(text, "zh");
    assert_eq!(t.segments.len(), 2);
    t.segments[0].end = 59.9996;
    assert!(segments_to_srt(&t).contains("00:01:00,000"));
}

#[test]
fn repeated_vtt_speech_at_different_times_is_preserved() {
    let t = parse_vtt(
        "WEBVTT\n\n00:00.000 --> 00:01.000\n重复\n\n00:02.000 --> 00:03.000\n重复\n",
        "zh",
    );
    assert_eq!(t.segments.len(), 2);
    assert_eq!(t.segments[1].start, 2.0);
}

#[cfg(unix)]
fn request(script: &str) -> avr_core::process::ProcessRequest {
    avr_core::process::ProcessRequest {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: None,
        timeout: Some(std::time::Duration::from_secs(2)),
        expected_outputs: vec![],
    }
}

#[cfg(unix)]
#[tokio::test]
async fn child_streams_are_drained_concurrently_past_retention_limit() {
    let result = avr_core::process::run(&request(
        "head -c 1500000 /dev/zero >&2; head -c 5000000 /dev/zero",
    ))
    .await;
    let out = result.unwrap();
    assert_eq!(out.stdout.len(), 4 * 1024 * 1024);
    assert_eq!(out.stderr.len(), 1024 * 1024);
}

#[cfg(unix)]
#[tokio::test]
async fn synchronous_compatibility_entry_works_inside_tokio() {
    let out = avr_core::process::run_blocking(&request("printf ok")).unwrap();
    assert_eq!(out.stdout, "ok");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_reaps_descendants() {
    let t = tempfile::tempdir().unwrap();
    let pid_file = t.path().join("pid");
    let mut req = request("sleep 30 & echo $! > pid; wait");
    req.cwd = Some(t.path().to_path_buf());
    let task = tokio::spawn(async move { avr_core::process::run(&req).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !pid_file.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match std::fs::read_to_string(format!("/proc/{}/stat", pid.trim())) {
                Err(_) => break,
                Ok(s) if s.split(") ").nth(1).unwrap().starts_with("Z ") => break,
                _ => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .unwrap();
}
