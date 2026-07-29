use std::path::PathBuf;
use tauri::Manager;

pub mod audit_seed;
pub mod commands;
pub mod connection;
pub mod db;
pub mod import_export;
pub mod plugin_registry;
pub mod rolling_restart;
pub mod settings_io;

fn db_file_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("bee-client.sqlite"))
}

fn legacy_settings_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("settings.json"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let db_path = db_file_path(app.handle()).unwrap_or_else(|| {
                std::env::temp_dir().join("bee-client.sqlite")
            });

            let database = db::Database::open(&db_path)
                .map_err(|e| format!("open db {}: {e}", db_path.display()))?;

            if let Some(json_path) = legacy_settings_path(app.handle()) {
                if let Err(e) = settings_io::import_legacy_addr(&database, &json_path) {
                    log::warn!("legacy settings import: {e}");
                }
            }

            app.manage(database);

            if let Err(e) = audit_seed::seed(app.state::<db::Database>().inner()) {
                log::warn!("audit seed: {e}");
            }

            let startup_addr = {
                let db = app.state::<db::Database>();
                let conn = db.lock().unwrap();
                db::settings::get(&conn, "addr")
                    .ok()
                    .flatten()
                    .unwrap_or_else(commands::connection::get_default_addr)
            };

            if let Ok(parsed) = connection::addr_parse(&startup_addr) {
                let _ = connection::ensure_bundle(parsed);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping::ping,
            commands::cluster::cluster_status,
            commands::cluster::list_jobs,
            commands::cluster::job_inspect,
            commands::connection::get_default_addr,
            commands::connection::set_addr,
            commands::connection::test_connection,
            commands::connection::conn_state,
            commands::settings::settings_get,
            commands::settings::settings_put,
            commands::settings::settings_list,
            commands::tabs::tabs_list,
            commands::tabs::tab_open,
            commands::tabs::tab_close,
            commands::tabs::tab_close_others,
            commands::tabs::tab_pin,
            commands::tabs::tab_set_active,
            commands::tabs::workspace_state,
            commands::profiles::profiles_list,
            commands::profiles::profile_save,
            commands::profiles::profile_remove,
            commands::applications::applications_list,
            commands::applications::application_create,
            commands::applications::application_set_enabled,
            commands::applications::application_enable,
            commands::applications::application_disable,
            commands::applications::application_delete,
            commands::audit::audit_list,
            commands::audit::audit_query,
            commands::audit::audit_latest,
            commands::audit::audit_record,
            commands::datasources::datasource_list,
            commands::datasources::datasource_create,
            commands::datasources::datasource_delete,
            commands::plugins::plugin_list,
            commands::plugins::plugin_schema,
            commands::rolling_restart::rolling_restart_apply,
            commands::pipelines::pipelines_list,
            commands::pipelines::pipeline_list,
            commands::pipelines::pipeline_create,
            commands::pipelines::pipeline_get,
            commands::pipelines::pipeline_delete,
            commands::pipelines::pipeline_latest_result,
            commands::applications::application_export,
            commands::applications::application_import,
            commands::search::search_local,
            commands::search::search_server,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
