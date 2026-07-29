use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::connection;
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct ClusterProfileView {
    pub id: i64,
    pub label: String,
    pub addr: String,
    pub tenant: u16,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn to_view(p: db::clusters::ClusterProfile) -> ClusterProfileView {
    ClusterProfileView {
        id: p.id,
        label: p.label,
        addr: p.addr,
        tenant: p.tenant,
        last_used_at: p.last_used_at,
        created_at: p.created_at,
    }
}

#[tauri::command]
pub fn cluster_profile_list(app: AppHandle) -> CmdResult<Vec<ClusterProfileView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::clusters::list(&conn)
        .map_err(CmdError::from)
        .map(|xs| xs.into_iter().map(to_view).collect())
}

#[tauri::command]
pub fn cluster_profile_save(
    app: AppHandle,
    label: String,
    addr: String,
    tenant: u16,
) -> CmdResult<i64> {
    let validated = crate::tenant::validate_tenant(tenant).map_err(CmdError::from)?;
    let _ = connection::addr_parse(&addr).map_err(CmdError::from)?;
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let id = db::clusters::save(&conn, &label, &addr, validated).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "cluster_profile.save",
        result: "Success",
        summary: &format!("Cluster profile \"{label}\" ({addr}) saved"),
        resource_kind: Some("cluster_profile"),
        resource_id: Some(&addr),
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("settings"),
        nav_resource_id: Some("cluster_profiles"),
    });
    Ok(id)
}

#[tauri::command]
pub fn cluster_profile_remove(app: AppHandle, addr: String) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::clusters::remove(&conn, &addr).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "cluster_profile.remove",
        result: "Success",
        summary: &format!("Cluster profile {addr} removed"),
        resource_kind: Some("cluster_profile"),
        resource_id: Some(&addr),
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("settings"),
        nav_resource_id: Some("cluster_profiles"),
    });
    Ok(())
}

#[tauri::command]
pub async fn cluster_profile_activate(
    app: AppHandle,
    addr: String,
) -> CmdResult<ClusterProfileView> {
    let parsed = connection::addr_parse(&addr).map_err(CmdError::from)?;
    {
        let db = db_handle(&app)?;
        let conn = db.lock().map_err(CmdError::from)?;
        db::settings::put(&conn, "addr", &addr).map_err(CmdError::from)?;
        db::clusters::set_active(&conn, &addr).map_err(CmdError::from)?;
        let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
            actor: "user",
            action: "cluster_profile.activate",
            result: "Success",
            summary: &format!("Active cluster switched to {addr}"),
            resource_kind: Some("cluster_profile"),
            resource_id: Some(&addr),
            application_id: None,
            correlation_id: None,
            operation_id: None,
            nav_kind: Some("settings"),
            nav_resource_id: Some("cluster_profiles"),
        });
    }
    let _ = connection::ensure_bundle(parsed);
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let profile = db::clusters::get(&conn, &addr)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("cluster_profile_activate: missing {addr}") })?;
    Ok(to_view(profile))
}

#[derive(Debug, serde::Deserialize)]
pub struct LegacyEntry {
    pub label: String,
    pub addr: String,
    pub tenant: Option<u16>,
}

#[derive(Debug, Serialize)]
pub struct MigrationReport {
    pub inserted: i64,
    pub skipped: Vec<String>,
}

#[tauri::command]
pub fn cluster_profile_migrate_legacy(
    app: AppHandle,
    entries: Vec<LegacyEntry>,
) -> CmdResult<MigrationReport> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let mut inserted = 0i64;
    let mut skipped: Vec<String> = Vec::new();
    for entry in entries {
        let parsed_addr = match connection::addr_parse(&entry.addr) {
            Ok(a) => a.to_string(),
            Err(_) => {
                skipped.push(entry.addr);
                continue;
            }
        };
        let tenant = entry.tenant.unwrap_or(0);
        if db::clusters::get(&conn, &parsed_addr).map_err(CmdError::from)?.is_some() {
            skipped.push(parsed_addr);
            continue;
        }
        db::clusters::save(&conn, &entry.label, &parsed_addr, tenant)
            .map_err(CmdError::from)?;
        inserted += 1;
    }
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "cluster_profile.migrate_legacy",
        result: "Success",
        summary: &format!(
            "Migrated {inserted} legacy cluster profile(s); skipped {}",
            skipped.len()
        ),
        resource_kind: Some("cluster_profile"),
        resource_id: None,
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("settings"),
        nav_resource_id: Some("cluster_profiles"),
    });
    Ok(MigrationReport { inserted, skipped })
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
    fn save_inserts_then_list_returns_it() {
        run(|db| {
            let conn = db.lock().unwrap();
            let id = db::clusters::save(&conn, "Local", "127.0.0.1:9999", 0).unwrap();
            let all = db::clusters::list(&conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].id, id);
            assert_eq!(all[0].addr, "127.0.0.1:9999");
            assert_eq!(all[0].tenant, 0);
        });
    }

    #[test]
    fn set_active_persists_addr_to_settings() {
        run(|db| {
            let conn = db.lock().unwrap();
            db::clusters::save(&conn, "Local", "127.0.0.1:9999", 0).unwrap();
            db::clusters::set_active(&conn, "127.0.0.1:9999").unwrap();
            db::settings::put(&conn, "addr", "127.0.0.1:9999").unwrap();
            let stored = db::settings::get(&conn, "addr").unwrap();
            assert_eq!(stored.as_deref(), Some("127.0.0.1:9999"));
        });
    }

    #[test]
    fn migrate_legacy_inserts_each_valid_entry() {
        run(|db| {
            let conn = db.lock().unwrap();
            let entries = vec![
                LegacyEntry {
                    label: "Local".into(),
                    addr: "127.0.0.1:9999".into(),
                    tenant: Some(0),
                },
                LegacyEntry {
                    label: "Staging".into(),
                    addr: "10.0.0.2:8000".into(),
                    tenant: Some(5),
                },
            ];
            let mut inserted = 0i64;
            let mut skipped: Vec<String> = Vec::new();
            for entry in entries {
                let parsed_addr = connection::addr_parse(&entry.addr).unwrap().to_string();
                let tenant = entry.tenant.unwrap_or(0);
                if db::clusters::get(&conn, &parsed_addr).unwrap().is_some() {
                    skipped.push(parsed_addr);
                    continue;
                }
                db::clusters::save(&conn, &entry.label, &parsed_addr, tenant).unwrap();
                inserted += 1;
            }
            assert_eq!(inserted, 2);
            assert!(skipped.is_empty());
            let all = db::clusters::list(&conn).unwrap();
            assert_eq!(all.len(), 2);
        });
    }

    #[test]
    fn migrate_legacy_skips_unparseable_and_duplicates() {
        run(|db| {
            let conn = db.lock().unwrap();
            db::clusters::save(&conn, "Local", "127.0.0.1:9999", 0).unwrap();
            let entries = vec![
                LegacyEntry {
                    label: "Existing".into(),
                    addr: "127.0.0.1:9999".into(),
                    tenant: Some(0),
                },
                LegacyEntry {
                    label: "Bad".into(),
                    addr: "not an addr".into(),
                    tenant: None,
                },
                LegacyEntry {
                    label: "Fresh".into(),
                    addr: "10.0.0.2:8000".into(),
                    tenant: Some(1),
                },
            ];
            let mut inserted = 0i64;
            let mut skipped: Vec<String> = Vec::new();
            for entry in entries {
                let parsed_addr = match connection::addr_parse(&entry.addr) {
                    Ok(a) => a.to_string(),
                    Err(_) => {
                        skipped.push(entry.addr);
                        continue;
                    }
                };
                let tenant = entry.tenant.unwrap_or(0);
                if db::clusters::get(&conn, &parsed_addr).unwrap().is_some() {
                    skipped.push(parsed_addr);
                    continue;
                }
                db::clusters::save(&conn, &entry.label, &parsed_addr, tenant).unwrap();
                inserted += 1;
            }
            assert_eq!(inserted, 1);
            assert_eq!(skipped.len(), 2);
        });
    }

    #[test]
    fn remove_then_list_omits_addr() {
        run(|db| {
            let conn = db.lock().unwrap();
            db::clusters::save(&conn, "Local", "127.0.0.1:9999", 0).unwrap();
            db::clusters::save(&conn, "Staging", "10.0.0.2:8000", 5).unwrap();
            db::clusters::remove(&conn, "127.0.0.1:9999").unwrap();
            let all = db::clusters::list(&conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].addr, "10.0.0.2:8000");
        });
    }
}