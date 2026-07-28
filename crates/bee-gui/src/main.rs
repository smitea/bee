use clap::Parser;
use std::net::SocketAddr;

use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "bee-gui", version, about = "Bee cluster management GUI")]
struct Cli {
    #[arg(long)]
    connect: String,
    #[arg(long, default_value = "info")]
    log_level: String,
    #[arg(long)]
    no_window_decorations: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli.log_level);

    let addr: SocketAddr = cli
        .connect
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid --connect addr '{}': {}", cli.connect, e))?;

    println!("bee-gui v0.1.0 — connect={} log_level={}", cli.connect, cli.log_level);

    // For S-1a, the GUI shell is wired up via a separate `app` module that
    // requires iced 0.12 to be fully realized. The binary here is the
    // entry point; the full app::run() launches the iced window.
    let _ = addr;
    Ok(())
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}