use std::process::Command;

#[test]
fn legacy_error_codes_and_explicit_mcp_root() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_scene-packet"))
            .arg("--data-root")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert_eq!(
        run(&["transcribe", "missing-file.mp4"]).status.code(),
        Some(2)
    );
    assert_eq!(
        run(&["analyze", "https://example.invalid/video"])
            .status
            .code(),
        Some(2)
    );
    let output = run(&["mcp-config"]);
    assert!(output.status.success(), "{output:?}");
    let data: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let args = data["mcpServers"]["scene-packet"]["args"]
        .as_array()
        .unwrap();
    assert_eq!(args[0], "--data-root");
    assert_eq!(
        std::path::Path::new(args[1].as_str().unwrap()),
        std::fs::canonicalize(dir.path()).unwrap()
    );
}
