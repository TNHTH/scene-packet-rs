//! 单 exe 入口：无参数 GUI / --mcp / CLI 子命令。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod console;
mod gui;
mod mcp;
mod state;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "avr", version, about = "ScenePacket - 视频上下文包")]
struct Args {
    /// 启动 MCP stdio 服务
    #[arg(long)]
    mcp: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 转写本地音视频文件
    Transcribe {
        path: String,
    },
    /// 分析 B站/抖音 URL
    Analyze {
        url: String,
    },
    /// 打印 MCP 配置 JSON
    McpConfig,
    /// 显示数据根与工具状态
    Doctor,
}

fn main() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    if args.mcp {
        if let Err(e) = mcp::run_mcp() {
            eprintln!("MCP 错误: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(cmd) = args.command {
        console::attach_console_if_needed();
        let code = match cmd {
            Commands::Transcribe { path } => cli::run_transcribe(&path),
            Commands::Analyze { url } => cli::run_analyze(&url),
            Commands::McpConfig => cli::run_mcp_config(),
            Commands::Doctor => cli::run_doctor(),
        };
        std::process::exit(code);
    }

    // 无参数 → GUI
    if let Err(e) = gui::run_gui() {
        eprintln!("启动 GUI 失败: {}", e);
        std::process::exit(1);
    }
}
