//! 桌面与无 GUI 二进制共享命令和 MCP 实现。
pub mod mcp;
use avr_core::{
    error::{process_failed, AvrError},
    pipeline::PipelineContext,
    root::resolve_data_root,
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
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
    let analyze = matches!(cmd, Commands::Analyze { .. });
    match execute(cmd) {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            0
        }
        Err(e) => {
            eprintln!("{}", e.format_user());
            if transcribe || (analyze && e.code == avr_core::ErrorCode::UnsupportedUrl) {
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
        return Ok(
            serde_json::json!({"mcpServers":{"scene-packet":{"command":exe,"args":["--data-root",context()?.root.path,"--mcp"]}}}),
        );
    }
    let ctx = context()?;
    if let Commands::Doctor { visual_only } = cmd {
        let registry = avr_core::tools::registry::ToolRegistry::new(&ctx.root, &ctx.config);
        let mut statuses = Vec::new();
        let mut ready = true;
        for name in ["ffmpeg", "ffprobe", "whisper", "yt-dlp"] {
            let status = registry.tool_status(name);
            let required =
                name == "ffmpeg" || name == "ffprobe" || (!visual_only && name == "whisper");
            if required && !status.ready {
                ready = false;
            }
            statuses.push(serde_json::json!({"tool":name,"ready":status.ready,"required":required,"path":status.path,"version":status.version}));
        }
        let asr_ready = avr_core::asr::engine::preset_ready(
            &ctx.root,
            &ctx.config,
            &ctx.config.asr.active_preset,
        );
        if !visual_only && !asr_ready {
            ready = false;
        }
        let value = serde_json::json!({"ready":ready,"data_root":ctx.root.path,"asr_ready":asr_ready,"tools":statuses});
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
                let r = avr_core::pipeline::transcribe_local(&ctx, &path).await?;
                Ok(avr_core::pipeline::compact_mcp_payload(&r, 3000))
            }
            Commands::Analyze { url } => {
                let r = avr_core::pipeline::analyze_url(&ctx, &url).await?;
                Ok(avr_core::pipeline::compact_mcp_payload(&r, 3000))
            }
            Commands::Frames {
                path,
                output,
                times,
                width,
            } => {
                let ff = avr_core::ffmpeg::FfmpegContext::resolve(&ctx.root, &ctx.config)?;
                let r = avr_core::frames::extract_frames(
                    &ff,
                    &path,
                    &times,
                    &output,
                    width,
                    avr_core::frames::FrameDelivery::Path,
                )
                .await?;
                serde_json::to_value(r).map_err(|e| process_failed(&e.to_string()))
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
