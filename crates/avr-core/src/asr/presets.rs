//! whisper.cpp 内置 preset。

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
}

pub fn is_supported(id: &str) -> bool {
    matches!(id, "whisper-cpp-cpu" | "whisper-cpp-cublas")
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

/// 过去 P4-1 默认使用的简体初始提示；现默认不传，仅作配置示例（`asr.zh_initial_prompt`）。
pub const ZH_INITIAL_PROMPT_EXAMPLE: &str = "以下是普通话的句子。";

/// ASR 参数修订号：会改变转写结果的参数 / 后处理变化时递增，并入缓存指纹使旧转写失效。
/// 2: P4-3 后处理；3: zh 改为默认不传提示词 + 转写后繁→简。
pub const ARGS_REV: u32 = 3;

impl Preset {
    /// `zh_prompt`：language=zh 时传的初始提示词，空串不传（默认）。
    pub fn args(
        &self,
        model: &Path,
        audio: &Path,
        language: &str,
        out_base: &str,
        zh_prompt: &str,
    ) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "-m".into(),
            model.display().to_string(),
            "-f".into(),
            audio.display().to_string(),
            "-l".into(),
            language.into(),
            "-osrt".into(),
            "-of".into(),
            out_base.into(),
        ];
        if language == "zh" && !zh_prompt.trim().is_empty() {
            args.push("--prompt".into());
            args.push(zh_prompt.into());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_for(lang: &str, prompt: &str) -> Vec<String> {
        get_preset("whisper-cpp-cpu").args(
            Path::new("m.bin"),
            Path::new("a.wav"),
            lang,
            "out",
            prompt,
        )
    }

    #[test]
    fn no_prompt_by_default() {
        for lang in ["zh", "en", "auto"] {
            assert!(
                !args_for(lang, "").iter().any(|x| x == "--prompt"),
                "{lang}"
            );
        }
    }

    #[test]
    fn configured_prompt_only_for_zh() {
        let a = args_for("zh", ZH_INITIAL_PROMPT_EXAMPLE);
        let i = a
            .iter()
            .position(|x| x == "--prompt")
            .expect("zh 配置后应传 --prompt");
        assert_eq!(a[i + 1], ZH_INITIAL_PROMPT_EXAMPLE);
        for lang in ["en", "ja", "auto", "zh-TW", "yue", ""] {
            assert!(
                !args_for(lang, ZH_INITIAL_PROMPT_EXAMPLE)
                    .iter()
                    .any(|x| x == "--prompt"),
                "{lang} 不应传 --prompt"
            );
        }
    }
}
