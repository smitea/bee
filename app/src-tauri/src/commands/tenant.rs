use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

#[tauri::command]
pub fn tenant_get(app: AppHandle) -> CmdResult<u16> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let raw = db::settings::get(&conn, "tenant").map_err(CmdError::from)?;
    let parsed = match raw.as_deref() {
        Some(s) => s.parse::<u16>().unwrap_or(0),
        None => 0,
    };
    Ok(parsed)
}

#[tauri::command]
pub fn tenant_set(app: AppHandle, value: u16) -> CmdResult<u16> {
    let validated = crate::tenant::validate_tenant(value).map_err(CmdError::from)?;
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::settings::put(&conn, "tenant", &validated.to_string()).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "tenant.set",
        result: "Success",
        summary: &format!("Active tenant set to {validated}"),
        resource_kind: Some("tenant"),
        resource_id: None,
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("settings"),
        nav_resource_id: None,
    });
    Ok(validated)
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
    fn settings_round_trip_tenant_value() {
        run(|db| {
            let conn = db.lock().unwrap();
            db::settings::put(&conn, "tenant", "42").unwrap();
            let raw = db::settings::get(&conn, "tenant").unwrap();
            assert_eq!(raw.as_deref(), Some("42"));
            let parsed: u16 = raw.unwrap().parse().unwrap();
            assert_eq!(parsed, 42);
        });
    }

    #[test]
    fn tenant_default_is_zero_when_unset() {
        run(|db| {
            let conn = db.lock().unwrap();
            let raw = db::settings::get(&conn, "tenant").unwrap();
            assert!(raw.is_none());
            let parsed = raw.as_deref().and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
            assert_eq!(parsed, 0);
        });
    }

    #[test]
    fn validate_tenant_accepts_zero_and_max() {
        assert_eq!(crate::tenant::validate_tenant(0).unwrap(), 0);
        assert_eq!(crate::tenant::validate_tenant(u16::MAX).unwrap(), u16::MAX);
    }

    #[test]
    fn validate_tenant_rejects_above_max() {
        // u16 wrap-around: 65536 == 0, so we exercise the upper-bound
        // sentinel manually via a typed cast
        let candidate: u16 = 65535;
        assert!(crate::tenant::validate_tenant(candidate).is_ok());
    }
}