//! 单 exe 入口：无参数 GUI / --mcp / CLI 子命令。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod console;
mod gui;
mod state;

use avr_cli::{run_command, Args};
use clap::Parser;
use tracing_subscriber::EnvFilter;

fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    if let Some(root) = &args.data_root {
        std::env::set_var("AVR_DATA_ROOT", root);
    }

    if args.mcp {
        if let Err(e) = avr_cli::mcp::run_mcp() {
            eprintln!("MCP 错误: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(cmd) = args.command {
        console::attach_console_if_needed();
        let code = run_command(cmd);
        std::process::exit(code);
    }

    // 无参数 → GUI
    if let Err(e) = gui::run_gui() {
        eprintln!("启动 GUI 失败: {}", e);
        std::process::exit(1);
    }
}
