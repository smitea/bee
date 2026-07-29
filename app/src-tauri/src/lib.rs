//! Bee GUI Tauri entry point.
//!
//! Two responsibilities:
//! 1. Spawn the connection thread on startup (uses `bee.toml` admin_addr
//!    or `127.0.0.1:9999` default).
//! 2. Register Tauri commands for the React frontend.

pub mod connection;
pub mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Bootstrap a connection on startup so the GUI can immediately
    // issue RPCs. Address: BEE_ADMIN_ADDR env var or 127.0.0.1:9999.
    let addr = std::env::var("BEE_ADMIN_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9999".to_string());
    if let Ok(parsed) = connection::addr_parse(&addr) {
        let _ = connection::ensure_bundle(parsed);
    }

    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::cluster_status,
            commands::list_jobs,
            commands::job_inspect,
            commands::connection_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}