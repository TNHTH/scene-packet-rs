//! Tauri GUI。

use crate::state::{AppState, SharedState};
use avr_core::config::{save_config, AppConfig};
use avr_core::jobs::JobProgress;
use avr_core::pipeline::{analyze, compact_mcp_payload, AnalyzeOptions, Source};
use avr_core::root::{needs_first_run_setup, set_root_pointer, DataRoot};
use avr_core::tools::install::download_and_install;
use avr_core::tools::manifest::{builtin_licenses, builtin_manifests};
use avr_core::tools::registry::ToolRegistry;
use avr_core::tools::ToolId;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};

#[derive(Serialize)]
struct ToolStatusDto {
    id: String,
    ready: bool,
    path: Option<String>,
    version: Option<String>,
    download_url: Option<String>,
}

#[derive(Serialize)]
struct LicenseDto {
    name: String,
    license: String,
    url: String,
}

pub fn run_gui() -> Result<(), String> {
    let state: SharedState = Arc::new(AppState::load().map_err(|e| e.format_user())?);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tauri::Builder::default()
            .plugin(tauri_plugin_dialog::init())
            .plugin(tauri_plugin_opener::init())
            .plugin(tauri_plugin_shell::init())
            .manage(state)
            .invoke_handler(tauri::generate_handler![
                needs_setup,
                get_data_root,
                set_data_root,
                get_config,
                save_app_config,
                list_tools,
                set_tool_path,
                install_tool,
                list_licenses,
                start_transcribe,
                start_analyze,
            ])
            .setup(|app| {
                #[cfg(windows)]
                apply_window_effects(app);
                Ok(())
            })
            .run(tauri::generate_context!())
            .map_err(|e| e.to_string())
    }));

    match result {
        Ok(r) => r,
        Err(_) => Err("应用发生未预期错误".into()),
    }
}

#[cfg(windows)]
fn apply_window_effects(app: &tauri::App) {
    use window_vibrancy::apply_mica;
    if let Some(window) = app.get_webview_window("main") {
        let _ = apply_mica(&window, None);
    }
}

#[tauri::command]
fn needs_setup() -> bool {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_ref().and_then(|p| p.parent());
    needs_first_run_setup(exe_dir)
}

#[tauri::command]
fn get_data_root(state: State<SharedState>) -> String {
    state.root.lock().path.display().to_string()
}

#[tauri::command]
fn set_data_root(path: String, state: State<SharedState>) -> Result<String, String> {
    let new_root = DataRoot {
        path: PathBuf::from(path),
    };
    new_root.ensure_layout().map_err(|e| e.format_user())?;
    set_root_pointer(&new_root.path).map_err(|e| e.format_user())?;
    let display = new_root.path.display().to_string();
    *state.root.lock() = new_root;
    Ok(display)
}

#[tauri::command]
fn get_config(state: State<SharedState>) -> AppConfig {
    state.config.lock().clone()
}

#[tauri::command]
fn save_app_config(
    cfg: AppConfig,
    app: tauri::AppHandle,
    state: State<SharedState>,
) -> Result<(), String> {
    let enable_mica = cfg.ui.enable_transparency;
    save_config(&state.root.lock(), &cfg).map_err(|e| e.format_user())?;
    *state.config.lock() = cfg;
    #[cfg(windows)]
    if enable_mica {
        if let Some(window) = app.get_webview_window("main") {
            use window_vibrancy::apply_mica;
            let _ = apply_mica(&window, None);
        }
    }
    Ok(())
}

#[tauri::command]
fn list_tools(state: State<SharedState>) -> Vec<ToolStatusDto> {
    let cfg = state.config.lock().clone();
    let reg = ToolRegistry::new(&state.root.lock(), &cfg);
    let manifests = builtin_manifests();
    ToolId::ALL
        .into_iter()
        .map(|id| {
            let st = reg.tool_status(id);
            let url = manifests
                .iter()
                .find(|m| id.matches_manifest(&m.id))
                .map(|m| m.url.clone());
            ToolStatusDto {
                id: id.to_string(),
                ready: st.ready,
                path: st.path.map(|p| p.display().to_string()),
                version: st.version,
                download_url: url,
            }
        })
        .collect()
}

#[tauri::command]
fn set_tool_path(tool_id: String, path: String, state: State<SharedState>) -> Result<(), String> {
    let mut cfg = state.config.lock().clone();
    match ToolId::parse(&tool_id).ok_or_else(|| "未知工具".to_string())? {
        ToolId::Ffmpeg => cfg.tools.ffmpeg_path = path,
        ToolId::Ffprobe => cfg.tools.ffprobe_path = path,
        ToolId::YtDlp => cfg.tools.ytdlp_path = path,
        ToolId::Whisper => cfg.tools.whisper_cli_path = path,
    }
    save_config(&state.root.lock(), &cfg).map_err(|e| e.format_user())?;
    *state.config.lock() = cfg;
    Ok(())
}

#[tauri::command]
async fn install_tool(tool_id: String, state: State<'_, SharedState>) -> Result<String, String> {
    let id = ToolId::parse(&tool_id).ok_or_else(|| "未知工具".to_string())?;
    let manifest = builtin_manifests()
        .into_iter()
        .find(|m| id.matches_manifest(&m.id))
        .ok_or_else(|| "未找到安装清单".to_string())?;
    let root_path = state.root.lock().path.clone();
    download_and_install(&manifest, &root_path)
        .await
        .map_err(|e| e.format_user())?;
    Ok(format!("已安装 {}", tool_id))
}

#[tauri::command]
fn list_licenses() -> Vec<LicenseDto> {
    builtin_licenses()
        .into_iter()
        .map(|(name, license, url)| LicenseDto {
            name: name.into(),
            license: license.into(),
            url: url.into(),
        })
        .collect()
}

#[tauri::command]
async fn start_transcribe(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let ctx = state.pipeline_ctx();
    let deps = state.deps().clone();
    let app_handle = app.clone();
    tokio::spawn(async move {
        let _ = app_handle.emit(
            "job-progress",
            JobProgress::Stage {
                media_key: path.clone(),
                stage: "starting".into(),
                percent: 0,
            },
        );
        let result = analyze(
            &ctx,
            &Source::Local(PathBuf::from(&path)),
            &AnalyzeOptions::default(),
            &deps,
        )
        .await;
        match result {
            Ok(r) => {
                let json = compact_mcp_payload(&r, 3000);
                let _ = app_handle.emit("job-done", json.to_string());
            }
            Err(e) => {
                let _ = app_handle.emit("job-error", e.format_user());
            }
        }
    });
    Ok("started".into())
}

#[tauri::command]
async fn start_analyze(
    url: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let ctx = state.pipeline_ctx();
    let deps = state.deps().clone();
    let app_handle = app.clone();
    tokio::spawn(async move {
        let result = analyze(&ctx, &Source::Url(url), &AnalyzeOptions::default(), &deps).await;
        match result {
            Ok(r) => {
                let json = compact_mcp_payload(&r, 3000);
                let _ = app_handle.emit("job-done", json.to_string());
            }
            Err(e) => {
                let _ = app_handle.emit("job-error", e.format_user());
            }
        }
    });
    Ok("started".into())
}
