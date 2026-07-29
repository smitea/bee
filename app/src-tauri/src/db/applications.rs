use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub display_order: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisableSnapshot {
    pub application_id: i64,
    pub taken_at: i64,
    pub payload_json: String,
}

pub fn list(conn: &Connection) -> Result<Vec<Application>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, enabled, display_order, created_at
             FROM applications
             ORDER BY display_order ASC, id ASC",
        )
        .map_err(|e| format!("applications.list prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Application {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                display_order: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| format!("applications.list query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("applications.list collect: {e}"))
}

pub fn next_display_order(conn: &Connection) -> Result<i64, String> {
    let order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(display_order), 0) + 1 FROM applications",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("applications.next_display_order: {e}"))?;
    Ok(order)
}

pub fn create(conn: &Connection, name: &str) -> Result<Application, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("applications.create: name must not be empty".into());
    }
    let order = next_display_order(conn)?;
    let now = now_secs();
    conn.execute(
        "INSERT INTO applications (name, enabled, display_order, created_at)
         VALUES (?, 1, ?, ?)",
        params![trimmed, order, now],
    )
    .map_err(|e| format!("applications.create({trimmed}): {e}"))?;
    let id = conn.last_insert_rowid();
    Ok(Application {
        id,
        name: trimmed.to_string(),
        enabled: true,
        display_order: order,
        created_at: now,
    })
}

pub fn set_enabled(conn: &Connection, id: i64, enabled: bool) -> Result<(), String> {
    let updated = conn
        .execute(
            "UPDATE applications SET enabled = ? WHERE id = ?",
            params![enabled as i64, id],
        )
        .map_err(|e| format!("applications.set_enabled({id}): {e}"))?;
    if updated == 0 {
        return Err(format!("applications.set_enabled: no row {id}"));
    }
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Application>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, enabled, display_order, created_at
             FROM applications
             WHERE id = ?",
        )
        .map_err(|e| format!("applications.get prepare: {e}"))?;
    let mut rows = stmt
        .query_map(params![id], |row| {
            Ok(Application {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                display_order: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| format!("applications.get query: {e}"))?;
    match rows.next() {
        Some(Ok(a)) => Ok(Some(a)),
        Some(Err(e)) => Err(format!("applications.get next: {e}")),
        None => Ok(None),
    }
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM applications WHERE id = ?", params![id])
        .map_err(|e| format!("applications.delete({id}): {e}"))
        .map(|_| ())
}

pub fn add_resource(
    conn: &Connection,
    application_id: i64,
    kind: &str,
    ref_id: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO application_resources (application_id, kind, ref_id) VALUES (?, ?, ?)",
        params![application_id, kind, ref_id],
    )
    .map_err(|e| format!("applications.add_resource: {e}"))
    .map(|_| ())
}

pub fn name_taken(conn: &Connection, name: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM applications WHERE name = ?",
            params![name],
            |row| row.get(0),
        )
        .map_err(|e| format!("applications.name_taken: {e}"))?;
    Ok(count > 0)
}

pub fn resources_for(
    conn: &Connection,
    application_id: i64,
) -> Result<Vec<(String, Option<String>)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, ref_id FROM application_resources
             WHERE application_id = ? ORDER BY kind, ref_id",
        )
        .map_err(|e| format!("applications.resources_for prepare: {e}"))?;
    let rows = stmt
        .query_map(params![application_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| format!("applications.resources_for query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("applications.resources_for collect: {e}"))
}

pub fn record_disable_snapshot(
    conn: &Connection,
    application_id: i64,
    payload_json: &str,
) -> Result<DisableSnapshot, String> {
    let taken_at = now_secs();
    conn.execute(
        "INSERT INTO application_disable_snapshots (application_id, taken_at, payload_json)
         VALUES (?, ?, ?)",
        params![application_id, taken_at, payload_json],
    )
    .map_err(|e| format!("applications.record_disable_snapshot({application_id}): {e}"))?;
    Ok(DisableSnapshot {
        application_id,
        taken_at,
        payload_json: payload_json.to_string(),
    })
}

pub fn list_disable_snapshots(
    conn: &Connection,
    application_id: i64,
) -> Result<Vec<DisableSnapshot>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT application_id, taken_at, payload_json
             FROM application_disable_snapshots
             WHERE application_id = ?
             ORDER BY taken_at DESC",
        )
        .map_err(|e| format!("applications.list_disable_snapshots prepare: {e}"))?;
    let rows = stmt
        .query_map(params![application_id], |row| {
            Ok(DisableSnapshot {
                application_id: row.get(0)?,
                taken_at: row.get(1)?,
                payload_json: row.get(2)?,
            })
        })
        .map_err(|e| format!("applications.list_disable_snapshots query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("applications.list_disable_snapshots collect: {e}"))
}

pub fn snapshot_payload(
    pipelines: &[crate::db::pipelines::PipelineDefinition],
    datasources: &[crate::db::datasources::Datasource],
) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct Payload<'a> {
        pipelines: Vec<PipelineRef<'a>>,
        datasources: Vec<DatasourceRef<'a>>,
    }
    #[derive(serde::Serialize)]
    struct PipelineRef<'a> {
        id: i64,
        name: &'a str,
    }
    #[derive(serde::Serialize)]
    struct DatasourceRef<'a> {
        name: &'a str,
        plugin: &'a str,
    }
    serde_json::to_string(&Payload {
        pipelines: pipelines
            .iter()
            .map(|p| PipelineRef {
                id: p.id,
                name: p.name.as_str(),
            })
            .collect(),
        datasources: datasources
            .iter()
            .map(|d| DatasourceRef {
                name: d.name.as_str(),
                plugin: d.plugin.as_str(),
            })
            .collect(),
    })
    .map_err(|e| format!("applications.snapshot_payload: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::tempdir;

    fn run<F: FnOnce(&Connection)>(f: F) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        let conn = db.lock().unwrap();
        f(&conn);
    }

    #[test]
    fn create_appends_to_list_with_monotonic_order() {
        run(|conn| {
            let a = create(conn, "alpha").unwrap();
            let b = create(conn, "beta").unwrap();
            assert!(a.display_order < b.display_order);
            let all = list(conn).unwrap();
            assert_eq!(all, vec![a.clone(), b.clone()]);
        });
    }

    #[test]
    fn create_rejects_empty_name() {
        run(|conn| {
            assert!(create(conn, "   ").is_err());
        });
    }

    #[test]
    fn set_enabled_toggles_flag() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            assert!(app.enabled);
            set_enabled(conn, app.id, false).unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all[0].enabled, false);
        });
    }

    #[test]
    fn set_enabled_is_idempotent_on_consecutive_calls() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            set_enabled(conn, app.id, true).unwrap();
            set_enabled(conn, app.id, true).unwrap();
            let all = list(conn).unwrap();
            assert!(all[0].enabled);
        });
    }

    #[test]
    fn get_returns_none_for_missing_id() {
        run(|conn| {
            assert!(get(conn, 9999).unwrap().is_none());
        });
    }

    #[test]
    fn get_round_trips_inserted_row() {
        run(|conn| {
            let a = create(conn, "alpha").unwrap();
            let fetched = get(conn, a.id).unwrap().unwrap();
            assert_eq!(fetched, a);
        });
    }

    #[test]
    fn delete_removes_row_and_referenced_resources_via_cascade() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            add_resource(conn, app.id, "dashboard", None).unwrap();
            delete(conn, app.id).unwrap();
            let all = list(conn).unwrap();
            assert!(all.is_empty());
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM application_resources WHERE application_id = ?",
                    params![app.id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn resources_for_returns_listed_pairs() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            add_resource(conn, app.id, "dashboard", None).unwrap();
            add_resource(conn, app.id, "pipeline", Some("p1")).unwrap();
            let resources = resources_for(conn, app.id).unwrap();
            assert_eq!(
                resources,
                vec![
                    ("dashboard".to_string(), None),
                    ("pipeline".to_string(), Some("p1".to_string())),
                ]
            );
        });
    }

    #[test]
    fn record_disable_snapshot_appends_each_time() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            let s1 = record_disable_snapshot(conn, app.id, r#"{"v":1}"#).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
            let s2 = record_disable_snapshot(conn, app.id, r#"{"v":2}"#).unwrap();
            assert!(s2.taken_at >= s1.taken_at);
            let snapshots = list_disable_snapshots(conn, app.id).unwrap();
            assert_eq!(snapshots.len(), 2);
            assert_eq!(snapshots[0].payload_json, r#"{"v":2}"#);
            assert_eq!(snapshots[1].payload_json, r#"{"v":1}"#);
        });
    }

    #[test]
    fn record_disable_snapshot_unknown_id_errors() {
        run(|conn| {
            assert!(record_disable_snapshot(conn, 9999, "{}").is_err());
        });
    }

    #[test]
    fn snapshot_payload_lists_pipelines_and_datasources_by_id_and_name() {
        let pipelines = vec![
            crate::db::pipelines::PipelineDefinition {
                id: 1,
                name: "alpha".into(),
                dag_json: "{}".into(),
                updated_at: 0,
            },
            crate::db::pipelines::PipelineDefinition {
                id: 2,
                name: "beta".into(),
                dag_json: "{}".into(),
                updated_at: 0,
            },
        ];
        let datasources = vec![crate::db::datasources::Datasource {
            name: "binance".into(),
            plugin: "binance_subscribe".into(),
            config: "{}".into(),
            tenant: 0,
            created_at: 0,
            updated_at: 0,
        }];
        let json = snapshot_payload(&pipelines, &datasources).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let ps = v.get("pipelines").unwrap().as_array().unwrap();
        let ds = v.get("datasources").unwrap().as_array().unwrap();
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].get("id").unwrap().as_i64(), Some(1));
        assert_eq!(ps[0].get("name").unwrap().as_str(), Some("alpha"));
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].get("name").unwrap().as_str(), Some("binance"));
        assert_eq!(
            ds[0].get("plugin").unwrap().as_str(),
            Some("binance_subscribe")
        );
    }
}