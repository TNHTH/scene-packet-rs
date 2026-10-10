//! CLI 子命令。

use crate::mcp::compact_mcp_payload;
use avr_core::config::load_config;
use avr_core::error::AvrError;

use crate::state::AppState;
use avr_core::root::resolve_data_root;
use avr_core::tools::registry::ToolRegistry;
use avr_core::tools::ToolId;

pub fn run_transcribe(path: &str) -> i32 {
    match run_transcribe_inner(path) {
        Ok(json) => {
            println!("{}", json);
            0
        }
        Err(e) => {
            eprintln!("{}", e.format_user());
            2
        }
    }
}

fn run_transcribe_inner(path: &str) -> Result<String, AvrError> {
    let state = AppState::load()?;
    let ctx = state.pipeline_ctx();
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(avr_core::pipeline::analyze(
            &ctx,
            &avr_core::pipeline::Source::Local(path.into()),
            &avr_core::pipeline::AnalyzeOptions::default(),
            state.deps(),
        ))?;
    Ok(serde_json::to_string_pretty(&compact_mcp_payload(&result, 3000)).unwrap())
}

pub fn run_analyze(url: &str) -> i32 {
    match run_analyze_inner(url) {
        Ok(json) => {
            println!("{}", json);
            0
        }
        Err(e) => {
            eprintln!("{}", e.format_user());
            if e.code == avr_core::ErrorCode::UnsupportedUrl {
                2
            } else {
                1
            }
        }
    }
}

fn run_analyze_inner(url: &str) -> Result<String, AvrError> {
    let state = AppState::load()?;
    let ctx = state.pipeline_ctx();
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(avr_core::pipeline::analyze(
            &ctx,
            &avr_core::pipeline::Source::Url(url.to_string()),
            &avr_core::pipeline::AnalyzeOptions::default(),
            state.deps(),
        ))?;
    Ok(serde_json::to_string_pretty(&compact_mcp_payload(&result, 3000)).unwrap())
}

pub fn run_mcp_config() -> i32 {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "avr".into());
    let json = serde_json::json!({
        "mcpServers": {
            "scene-packet": {
                "command": exe,
                "args": ["--mcp"]
            }
        }
    });
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
    0
}

pub fn run_doctor() -> i32 {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_ref().and_then(|p| p.parent());
    match resolve_data_root(exe_dir) {
        Ok(root) => {
            println!("数据根: {}", root.path.display());
            root.ensure_layout().ok();
            if let Ok(cfg) = load_config(&root) {
                let reg = ToolRegistry::new(&root, &cfg);
                for id in ToolId::ALL {
                    let st = reg.tool_status(id);
                    println!(
                        "  {}: {} {}",
                        id,
                        if st.ready { "OK" } else { "MISSING" },
                        st.version.unwrap_or_default()
                    );
                }
            }
            0
        }
        Err(e) => {
            eprintln!("{}", e.format_user());
            1
        }
    }
}
