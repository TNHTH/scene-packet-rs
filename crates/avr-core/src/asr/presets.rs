//! whisper.cpp 内置 preset。

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
}

pub fn get_preset(id: &str) -> Preset {
    match id {
        "whisper-cpp-cublas" => Preset {
            id: "whisper-cpp-cublas",
            label: "Whisper.cpp cuBLAS",
        },
        _ => Preset {
            id: "whisper-cpp-cpu",
            label: "Whisper.cpp CPU",
        },
    }
}

pub fn default_model_path(root_models: &Path) -> PathBuf {
    root_models.join("ggml-large-v3-turbo.bin")
}

impl Preset {
    pub fn args(&self, model: &Path, audio: &Path, language: &str, out_base: &str) -> Vec<String> {
        vec![
            "-m".into(),
            model.display().to_string(),
            "-f".into(),
            audio.display().to_string(),
            "-l".into(),
            language.into(),
            "-osrt".into(),
            "-of".into(),
            out_base.into(),
        ]
    }
}
