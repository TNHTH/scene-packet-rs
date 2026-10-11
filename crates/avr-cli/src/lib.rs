//! 无界面命令。MCP 服务本身仍由 `avr-mcp` 提供。

pub mod mcp;

use std::path::PathBuf;
use std::sync::Arc;

use avr_core::error::{process_failed, AvrError};
use avr_core::jobs::JobManager;
use avr_core::pipeline::{analyze, AnalyzeOptions, Deps, PipelineContext, Source};
use avr_core::root::resolve_data_root;
use avr_core::tools::registry::ToolRegistry;
use avr_core::tools::ToolId;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "scene-packet",
    version,
    about = "ScenePacket：字幕、全片截图和可追溯图文素材包"
)]
pub struct Args {
    #[arg(long)]
    pub mcp: bool,
    #[arg(long, global = true)]
    pub data_root: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    Transcribe {
        path: PathBuf,
    },
    Analyze {
        url: String,
    },
    McpConfig,
    Doctor {
        #[arg(long)]
        visual_only: bool,
    },
    Frames {
        path: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, value_delimiter = ',', required = true)]
        times: Vec<f64>,
        #[arg(long, default_value_t = 1280)]
        width: u32,
    },
    Packet {
        path: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 2.0)]
        interval: f64,
        #[arg(long, default_value_t = 120)]
        max_frames: usize,
        #[arg(long, value_delimiter = ',')]
        times: Vec<f64>,
        #[arg(long)]
        visual_only: bool,
    },
}

pub fn context() -> Result<PipelineContext, AvrError> {
    let exe = std::env::current_exe().ok();
    PipelineContext::load(resolve_data_root(exe.as_ref().and_then(|p| p.parent()))?)
}

pub fn run_command(cmd: Commands) -> i32 {
    let transcribe = matches!(cmd, Commands::Transcribe { .. });
    let analyze_cmd = matches!(cmd, Commands::Analyze { .. });
    match execute(cmd) {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            0
        }
        Err(e) => {
            eprintln!("{}", e.format_user());
            if transcribe || (analyze_cmd && e.code == avr_core::ErrorCode::UnsupportedUrl) {
                2
            } else {
                1
            }
        }
    }
}

fn execute(cmd: Commands) -> Result<serde_json::Value, AvrError> {
    if matches!(cmd, Commands::McpConfig) {
        let exe = std::env::current_exe().map_err(|e| process_failed(&e.to_string()))?;
        let root = context()?.root.path;
        let shown = std::fs::canonicalize(&root).unwrap_or(root);
        return Ok(serde_json::json!({
            "mcpServers": {
                "scene-packet": {
                    "command": exe,
                    "args": ["--data-root", shown, "--mcp"]
                }
            }
        }));
    }
    let ctx = context()?;
    if let Commands::Doctor { visual_only } = cmd {
        let registry = ToolRegistry::new(&ctx.root, &ctx.config);
        let mut statuses = Vec::new();
        let mut ready = true;
        for id in ToolId::ALL {
            let status = registry.tool_status(id);
            let required = matches!(id, ToolId::Ffmpeg | ToolId::Ffprobe)
                || (!visual_only && matches!(id, ToolId::Whisper));
            if required && !status.ready {
                ready = false;
            }
            statuses.push(serde_json::json!({
                "tool": id.as_str(),
                "ready": status.ready,
                "required": required,
                "path": status.path,
                "version": status.version
            }));
        }
        let value = serde_json::json!({
            "ready": ready,
            "data_root": ctx.root.path,
            "tools": statuses
        });
        if !ready {
            return Err(process_failed(&format!("依赖检查未通过: {value}")));
        }
        return Ok(value);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| process_failed(&e.to_string()))?;
    runtime.block_on(async {
        match cmd {
            Commands::Transcribe { path } => {
                let result = analyze(
                    &ctx,
                    &Source::Local(path),
                    &AnalyzeOptions::default(),
                    &Deps::new(Arc::new(JobManager::new())),
                )
                .await?;
                serde_json::to_value(result).map_err(|e| process_failed(&e.to_string()))
            }
            Commands::Analyze { url } => {
                let result = analyze(
                    &ctx,
                    &Source::Url(url),
                    &AnalyzeOptions::default(),
                    &Deps::new(Arc::new(JobManager::new())),
                )
                .await?;
                serde_json::to_value(result).map_err(|e| process_failed(&e.to_string()))
            }
            Commands::Frames {
                path,
                output,
                times,
                width,
            } => {
                let ff = avr_core::ffmpeg::FfmpegContext::resolve(&ctx.root, &ctx.config)?;
                let frames = avr_core::frames::extract_frames(
                    &ff,
                    &path,
                    &times,
                    &output,
                    width,
                    avr_core::frames::FrameDelivery::Path,
                )
                .await?;
                serde_json::to_value(frames).map_err(|e| process_failed(&e.to_string()))
            }
            Commands::Packet {
                path,
                output,
                interval,
                max_frames,
                times,
                visual_only,
            } => {
                avr_core::packet::build_packet(
                    &ctx,
                    &path,
                    &output,
                    &avr_core::packet::PacketOptions {
                        interval,
                        max_frames,
                        times,
                        visual_only,
                    },
                )
                .await
            }
            _ => unreachable!(),
        }
    })
}
