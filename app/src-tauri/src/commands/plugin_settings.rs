use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct PluginSettingView {
    pub plugin_name: String,
    pub enabled: bool,
    pub config_json: String,
    pub updated_at: i64,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn to_view(p: db::plugin_settings::PluginSetting) -> PluginSettingView {
    PluginSettingView {
        plugin_name: p.plugin_name,
        enabled: p.enabled,
        config_json: p.config_json,
        updated_at: p.updated_at,
    }
}

#[tauri::command]
pub fn plugin_settings_get(
    app: AppHandle,
    name: String,
) -> CmdResult<Option<PluginSettingView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::plugin_settings::get(&conn, &name)
        .map_err(CmdError::from)
        .map(|opt| opt.map(to_view))
}

#[tauri::command]
pub fn plugin_settings_set(
    app: AppHandle,
    name: String,
    enabled: bool,
    config_json: String,
) -> CmdResult<PluginSettingView> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let saved = db::plugin_settings::upsert(&conn, &name, enabled, &config_json)
        .map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "plugin_settings.set",
        result: "Success",
        summary: &format!(
            "Plugin \"{name}\" enabled={enabled}"
        ),
        resource_kind: Some("plugin"),
        resource_id: Some(&name),
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("settings"),
        nav_resource_id: Some("plugin_settings"),
    });
    Ok(to_view(saved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::tempdir;

    fn run<F: FnOnce(&Database)>(f: F) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        f(&db);
    }

    #[test]
    fn upsert_then_get_round_trips_via_db_repo() {
        run(|db| {
            let conn = db.lock().unwrap();
            let saved = db::plugin_settings::upsert(&conn, "binance", true, r#"{"url":"wss"}"#).unwrap();
            assert_eq!(saved.plugin_name, "binance");
            let fetched = db::plugin_settings::get(&conn, "binance").unwrap().unwrap();
            assert_eq!(fetched.enabled, true);
            assert_eq!(fetched.config_json, r#"{"url":"wss"}"#);
        });
    }

    #[test]
    fn to_view_extracts_all_fields() {
        let p = db::plugin_settings::PluginSetting {
            plugin_name: "alpha".into(),
            enabled: false,
            config_json: r#"{"x":1}"#.into(),
            updated_at: 42,
        };
        let v = to_view(p);
        assert_eq!(v.plugin_name, "alpha");
        assert_eq!(v.enabled, false);
        assert_eq!(v.config_json, r#"{"x":1}"#);
        assert_eq!(v.updated_at, 42);
    }
}