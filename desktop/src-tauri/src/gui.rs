//! Tauri GUI。

use crate::error::{CommandError, CommandResult};
use crate::mcp::compact_mcp_payload;
use crate::state::{AppState, SharedState};
use avr_core::config::{load_config, save_config, AppConfig};
use avr_core::pipeline::{analyze, AnalyzeOptions, Source};
use avr_core::root::{needs_first_run_setup, set_root_pointer, DataRoot};
use avr_core::tools::install::download_and_install;
use avr_core::tools::manifest::{builtin_licenses, builtin_manifests};
use avr_core::tools::registry::ToolRegistry;
use avr_core::tools::ToolId;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(windows)]
use tauri::Manager;
use tauri::{Emitter, State};

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
            .setup(|_app| {
                #[cfg(windows)]
                apply_window_effects(_app);
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
    state.context.lock().root.path.display().to_string()
}

#[tauri::command]
fn set_data_root(path: String, state: State<SharedState>) -> CommandResult<String> {
    let new_root = DataRoot {
        path: PathBuf::from(path),
    };
    new_root.ensure_layout()?;
    let config = load_config(&new_root)?;
    let mut context = state.context.lock();
    set_root_pointer(&new_root.path)?;
    let display = new_root.path.display().to_string();
    *context = avr_core::pipeline::PipelineContext {
        root: new_root,
        config,
    };
    Ok(display)
}

#[tauri::command]
fn get_config(state: State<SharedState>) -> AppConfig {
    state.context.lock().config.clone()
}

#[tauri::command]
fn save_app_config(
    cfg: AppConfig,
    _app: tauri::AppHandle,
    state: State<SharedState>,
) -> CommandResult<()> {
    #[cfg(windows)]
    let enable_mica = cfg.ui.enable_transparency;
    let mut context = state.context.lock();
    save_config(&context.root, &cfg)?;
    context.config = cfg;
    #[cfg(windows)]
    if enable_mica {
        if let Some(window) = _app.get_webview_window("main") {
            use window_vibrancy::apply_mica;
            let _ = apply_mica(&window, None);
        }
    }
    Ok(())
}

#[tauri::command]
async fn list_tools(state: State<'_, SharedState>) -> CommandResult<Vec<ToolStatusDto>> {
    let context = state.pipeline_ctx();
    let reg = ToolRegistry::new(&context.root, &context.config);
    let manifests = builtin_manifests();
    let (a, b, c, d) = tokio::join!(
        reg.tool_status_async(ToolId::Ffmpeg),
        reg.tool_status_async(ToolId::Ffprobe),
        reg.tool_status_async(ToolId::YtDlp),
        reg.tool_status_async(ToolId::Whisper)
    );
    Ok([a, b, c, d]
        .into_iter()
        .map(|st| ToolStatusDto {
            id: st.id.to_string(),
            ready: st.ready,
            path: st.path.map(|p| p.display().to_string()),
            version: st.version,
            download_url: if cfg!(windows) {
                manifests
                    .iter()
                    .find(|m| st.id.matches_manifest(&m.id))
                    .map(|m| m.url.clone())
            } else {
                None
            },
        })
        .collect())
}

#[tauri::command]
fn set_tool_path(tool_id: String, path: String, state: State<SharedState>) -> CommandResult<()> {
    let mut context = state.context.lock();
    let mut cfg = context.config.clone();
    match ToolId::parse(&tool_id).ok_or_else(|| CommandError::invalid_argument("未知工具"))? {
        ToolId::Ffmpeg => cfg.tools.ffmpeg_path = path,
        ToolId::Ffprobe => cfg.tools.ffprobe_path = path,
        ToolId::YtDlp => cfg.tools.ytdlp_path = path,
        ToolId::Whisper => cfg.tools.whisper_cli_path = path,
    }
    save_config(&context.root, &cfg)?;
    context.config = cfg;
    Ok(())
}

#[tauri::command]
async fn install_tool(tool_id: String, state: State<'_, SharedState>) -> CommandResult<String> {
    if !cfg!(windows) {
        return Err(CommandError::invalid_argument(
            "内置下载仅支持 Windows；请配置本机工具路径",
        ));
    }
    let id = ToolId::parse(&tool_id).ok_or_else(|| CommandError::invalid_argument("未知工具"))?;
    let manifest = builtin_manifests()
        .into_iter()
        .find(|m| id.matches_manifest(&m.id))
        .ok_or_else(|| CommandError::invalid_argument("未找到安装清单"))?;
    let root_path = state.context.lock().root.path.clone();
    download_and_install(&manifest, &root_path).await?;
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
    Ok(start_job(Source::Local(PathBuf::from(path)), app, &state))
}

#[tauri::command]
async fn start_analyze(
    url: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    Ok(start_job(Source::Url(url), app, &state))
}

fn start_job(source: Source, app: tauri::AppHandle, state: &SharedState) -> String {
    let ctx = state.pipeline_ctx();
    let deps = state.deps().clone();
    let mut events = deps.jobs().subscribe();
    tokio::spawn(async move {
        let options = AnalyzeOptions::default();
        let analysis = analyze(&ctx, &source, &options, &deps);
        tokio::pin!(analysis);
        let result = loop {
            tokio::select! {
                result = &mut analysis => break result,
                Ok(event) = events.recv() => { let _ = app.emit("job-progress", event); }
            }
        };
        match result {
            Ok(r) => {
                let _ = app.emit("job-done", compact_mcp_payload(&r, 3000).to_string());
            }
            Err(e) => {
                let _ = app.emit("job-error", e.format_user());
            }
        }
    });
    "started".into()
}
