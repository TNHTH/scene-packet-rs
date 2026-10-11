use clap::Parser;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = avr_cli::Args::parse();
    if let Some(root) = &args.data_root {
        std::env::set_var("AVR_DATA_ROOT", root);
    }
    if args.mcp {
        if let Err(e) = avr_cli::mcp::run_mcp() {
            eprintln!("{}", e.format_user());
            std::process::exit(1);
        }
        return;
    }
    if let Some(cmd) = args.command {
        std::process::exit(avr_cli::run_command(cmd));
    }
    use clap::CommandFactory;
    let _ = avr_cli::Args::command().print_help();
}
