use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "bee-gui", version, about = "Bee cluster management GUI")]
struct Cli {
    /// Admin server address (e.g. 127.0.0.1:10001)
    #[arg(long)]
    connect: String,

    /// Log level (debug|info|warn|error)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// macOS: hide traffic-light buttons
    #[arg(long)]
    no_window_decorations: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    println!(
        "bee-gui v0.1.0 — connect={} log_level={}",
        cli.connect, cli.log_level
    );
    Ok(())
}