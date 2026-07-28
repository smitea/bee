//! bee-gui entry point — launches the iced window.

use std::net::SocketAddr;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use bee_gui::app::{App, Flags};
use iced::Application;
use bee_gui::connection::spawn;
use bee_gui::log_panel::LogRing;

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

    let bundle = spawn(addr);
    let log = LogRing::new();
    let flags = Flags { bundle, log };

    let settings = iced::Settings::<Flags> {
        id: None,
        window: iced::window::Settings {
            size: iced::Size::new(1100.0, 720.0),
            decorations: !cli.no_window_decorations,
            ..Default::default()
        },
        flags,
        fonts: Default::default(),
        default_font: iced::Font::default(),
        antialiasing: true,
        default_text_size: iced::Pixels(13.0),
    };

    let _ = App::run(settings);
    Ok(())
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}