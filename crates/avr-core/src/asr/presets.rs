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

/// language=zh 时传给 whisper 的简体初始提示（P4-1）。whisper medium/large 对 zh
/// 常输出繁体；用简体提示把输出推向简体写法，不引入繁简转换依赖。
pub const ZH_INITIAL_PROMPT: &str = "以下是普通话的句子。";

/// ASR 参数修订号：会改变转写结果的参数变化时递增，并入缓存指纹使旧转写失效。
pub const ARGS_REV: u32 = 2; // 2: P4-3 后处理

impl Preset {
    pub fn args(&self, model: &Path, audio: &Path, language: &str, out_base: &str) -> Vec<String> {
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
        if language == "zh" {
            args.push("--prompt".into());
            args.push(ZH_INITIAL_PROMPT.into());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_for(lang: &str) -> Vec<String> {
        get_preset("whisper-cpp-cpu").args(Path::new("m.bin"), Path::new("a.wav"), lang, "out")
    }

    #[test]
    fn zh_passes_simplified_prompt() {
        let a = args_for("zh");
        let i = a
            .iter()
            .position(|x| x == "--prompt")
            .expect("zh 应传 --prompt");
        assert_eq!(a[i + 1], ZH_INITIAL_PROMPT);
        assert_eq!(a.iter().filter(|x| *x == "--prompt").count(), 1);
    }

    #[test]
    fn non_zh_has_no_prompt() {
        for lang in ["en", "ja", "auto", "zh-TW", "yue", ""] {
            assert!(
                !args_for(lang).iter().any(|x| x == "--prompt"),
                "{lang} 不应传 --prompt"
            );
        }
    }
}
