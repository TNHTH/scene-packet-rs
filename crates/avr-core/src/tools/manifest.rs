//! 工具 manifest 定义。

use serde::{Deserialize, Serialize};

use crate::tools::ToolId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolManifest {
    pub id: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub license: String,
    pub license_url: String,
    pub source_url: String,
    pub install_dir: String,
    pub archive_layout: ArchiveLayout,
    pub depends: Vec<String>,
    pub optional: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveLayout {
    SingleExe { path: String },
    ZipExtract { entries: Vec<String> },
}

pub fn builtin_manifests() -> Vec<ToolManifest> {
    vec![
        ToolManifest {
            id: ToolId::Ffmpeg.as_str().into(),
            version: "essentials".into(),
            url: "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip".into(),
            sha256: String::new(),
            license: "GPLv3".into(),
            license_url: "https://www.gyan.dev/ffmpeg/builds/".into(),
            source_url: "https://ffmpeg.org/".into(),
            install_dir: "bin/ffmpeg".into(),
            archive_layout: ArchiveLayout::ZipExtract {
                entries: vec!["ffmpeg.exe".into(), "ffprobe.exe".into()],
            },
            depends: vec![],
            optional: vec![],
        },
        ToolManifest {
            id: ToolId::YtDlp.as_str().into(),
            version: "latest".into(),
            url: "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe".into(),
            sha256: String::new(),
            license: "Unlicense".into(),
            license_url: "https://github.com/yt-dlp/yt-dlp".into(),
            source_url: "https://github.com/yt-dlp/yt-dlp".into(),
            install_dir: "bin/yt-dlp".into(),
            archive_layout: ArchiveLayout::SingleExe {
                path: "yt-dlp.exe".into(),
            },
            depends: vec![],
            optional: vec!["yt-dlp-ejs".into(), "js-runtime".into()],
        },
        ToolManifest {
            id: "whisper-cpp-cpu".into(),
            version: "1.9.0".into(),
            url: "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.0/whisper-bin-x64.zip"
                .into(),
            sha256: String::new(),
            license: "MIT".into(),
            license_url: "https://github.com/ggml-org/whisper.cpp".into(),
            source_url: "https://github.com/ggml-org/whisper.cpp".into(),
            install_dir: "bin/whisper".into(),
            archive_layout: ArchiveLayout::ZipExtract {
                entries: vec!["whisper-cli.exe".into(), "ggml.dll".into()],
            },
            depends: vec![],
            optional: vec![],
        },
        ToolManifest {
            id: "whisper-cpp-cublas".into(),
            version: "1.9.0".into(),
            url: "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.0/whisper-cublas-11.8.0-bin-x64.zip"
                .into(),
            sha256: String::new(),
            license: "MIT".into(),
            license_url: "https://github.com/ggml-org/whisper.cpp".into(),
            source_url: "https://github.com/ggml-org/whisper.cpp".into(),
            install_dir: "bin/whisper".into(),
            archive_layout: ArchiveLayout::ZipExtract {
                entries: vec!["whisper-cli.exe".into(), "ggml-cuda.dll".into()],
            },
            depends: vec![],
            optional: vec![],
        },
    ]
}

/// 第三方许可摘要（About 页）。
pub fn builtin_licenses() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("ffmpeg", "GPLv3", "https://ffmpeg.org/"),
        ("yt-dlp", "Unlicense", "https://github.com/yt-dlp/yt-dlp"),
        (
            "whisper.cpp",
            "MIT",
            "https://github.com/ggml-org/whisper.cpp",
        ),
    ]
}
