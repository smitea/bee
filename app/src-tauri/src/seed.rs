use rusqlite::Connection;

use crate::db::{self, Database, audit::NewAuditEvent};

pub const DEMO_APP_NAME: &str = "Demo";
pub const DEMO_PIPELINE_NAME: &str = "kline-sample";
pub const DEMO_DATASOURCE_NAME: &str = "kline-sample";
pub const DEMO_PLUGIN_NAME: &str = "kline_subscribe";

pub const DEMO_DAG_JSON: &str = r#"{
  "name": "kline-sample",
  "phases": [
    {
      "id": "kline-sample.kline",
      "kind": "input",
      "datasource": "kline-sample",
      "method": "kline"
    },
    {
      "id": "ema",
      "kind": "handler",
      "inputs": ["kline-sample.kline"],
      "outputs": ["sample-kline.emit"]
    },
    {
      "id": "sample-kline.emit",
      "kind": "output",
      "datasource": "kline-sample",
      "method": "emit"
    }
  ],
  "edges": [
    {"from": "kline-sample.kline", "to": "ema"},
    {"from": "ema", "to": "sample-kline.emit"}
  ]
}"#;

pub const DEMO_DATASOURCE_CONFIG: &str =
    r#"{"url":"sample","symbol":"BTC/USDT","interval":"5min"}"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedReport {
    pub created: bool,
    pub application_id: Option<i64>,
    pub pipeline_id: Option<i64>,
    pub datasource_name: Option<String>,
    pub audit_events: usize,
}

impl SeedReport {
    pub fn not_seeded() -> Self {
        Self {
            created: false,
            application_id: None,
            pipeline_id: None,
            datasource_name: None,
            audit_events: 0,
        }
    }
}

pub fn seed_demo(conn: &Connection) -> Result<SeedReport, String> {
    let app_exists = db::applications::name_taken(conn, DEMO_APP_NAME)?;
    let pipeline_exists = db::pipelines::name_taken(conn, DEMO_PIPELINE_NAME)?;
    let datasource_exists = db::datasources::name_taken(conn, DEMO_DATASOURCE_NAME)?;
    if app_exists && pipeline_exists && datasource_exists {
        return Ok(SeedReport::not_seeded());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("seed_demo: begin tx: {e}"))?;

    let app = if !db::applications::name_taken(&tx, DEMO_APP_NAME)? {
        db::applications::create(&tx, DEMO_APP_NAME)?
    } else {
        db::applications::list(&tx)?
            .into_iter()
            .find(|a| a.name == DEMO_APP_NAME)
            .ok_or_else(|| format!("seed_demo: missing existing application {DEMO_APP_NAME}"))?
    };

    let pipeline = if !db::pipelines::name_taken(&tx, DEMO_PIPELINE_NAME)? {
        db::pipelines::create(&tx, DEMO_PIPELINE_NAME, DEMO_DAG_JSON)?
    } else {
        db::pipelines::list(&tx)?
            .into_iter()
            .find(|p| p.name == DEMO_PIPELINE_NAME)
            .ok_or_else(|| format!("seed_demo: missing existing pipeline {DEMO_PIPELINE_NAME}"))?
    };

    let datasource = if !db::datasources::name_taken(&tx, DEMO_DATASOURCE_NAME)? {
        db::datasources::create(
            &tx,
            DEMO_DATASOURCE_NAME,
            DEMO_PLUGIN_NAME,
            DEMO_DATASOURCE_CONFIG,
            0,
        )?
    } else {
        db::datasources::list(&tx)?
            .into_iter()
            .find(|d| d.name == DEMO_DATASOURCE_NAME)
            .ok_or_else(|| {
                format!("seed_demo: missing existing datasource {DEMO_DATASOURCE_NAME}")
            })?
    };

    db::applications::add_resource(&tx, app.id, "pipeline", Some(DEMO_PIPELINE_NAME))?;
    db::applications::add_resource(&tx, app.id, "datasource", Some(DEMO_DATASOURCE_NAME))?;

    let summary_app = format!("Application \"{}\" seeded", app.name);
    db::audit::record(
        &tx,
        NewAuditEvent {
            actor: "bee-client",
            action: "application.create",
            result: "Success",
            summary: &summary_app,
            resource_kind: Some("application"),
            resource_id: Some(&app.name),
            application_id: Some(app.id),
            correlation_id: None,
            operation_id: None,
            nav_kind: Some("application"),
            nav_resource_id: Some(&app.name),
        },
    )?;

    let summary_pipeline = format!("Pipeline \"{}\" seeded", pipeline.name);
    db::audit::record(
        &tx,
        NewAuditEvent {
            actor: "bee-client",
            action: "pipeline.create",
            result: "Success",
            summary: &summary_pipeline,
            resource_kind: Some("pipeline"),
            resource_id: Some(&pipeline.name),
            application_id: Some(app.id),
            correlation_id: None,
            operation_id: None,
            nav_kind: Some("pipeline"),
            nav_resource_id: Some(&pipeline.name),
        },
    )?;

    let summary_ds = format!("Datasource \"{}\" seeded", datasource.name);
    db::audit::record(
        &tx,
        NewAuditEvent {
            actor: "bee-client",
            action: "datasource.create",
            result: "Success",
            summary: &summary_ds,
            resource_kind: Some("datasource"),
            resource_id: Some(&datasource.name),
            application_id: Some(app.id),
            correlation_id: None,
            operation_id: None,
            nav_kind: Some("datasource"),
            nav_resource_id: Some(&datasource.name),
        },
    )?;

    let summary_demo = "Seeded demo application with pipeline and datasource".to_string();
    db::audit::record(
        &tx,
        NewAuditEvent {
            actor: "bee-client",
            action: "demo.seed",
            result: "Success",
            summary: &summary_demo,
            resource_kind: None,
            resource_id: None,
            application_id: Some(app.id),
            correlation_id: None,
            operation_id: None,
            nav_kind: Some("application"),
            nav_resource_id: Some(&app.name),
        },
    )?;

    tx.commit()
        .map_err(|e| format!("seed_demo: commit: {e}"))?;

    Ok(SeedReport {
        created: true,
        application_id: Some(app.id),
        pipeline_id: Some(pipeline.id),
        datasource_name: Some(datasource.name),
        audit_events: 4,
    })
}

pub fn seed_demo_db(db: &Database) -> Result<SeedReport, String> {
    let conn = db.lock().map_err(|e| format!("seed_demo_db.lock: {e}"))?;
    seed_demo(&conn)
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
    fn seed_demo_creates_application_pipeline_datasource_and_audit_events() {
        run(|conn| {
            let report = seed_demo(conn).unwrap();
            assert!(report.created);
            assert!(report.application_id.is_some());
            assert!(report.pipeline_id.is_some());
            assert_eq!(report.datasource_name.as_deref(), Some(DEMO_DATASOURCE_NAME));
            assert_eq!(report.audit_events, 4);

            let apps = db::applications::list(conn).unwrap();
            assert_eq!(apps.len(), 1);
            assert_eq!(apps[0].name, DEMO_APP_NAME);

            let pipelines = db::pipelines::list(conn).unwrap();
            assert_eq!(pipelines.len(), 1);
            assert_eq!(pipelines[0].name, DEMO_PIPELINE_NAME);
            assert!(pipelines[0].dag_json.contains("kline-sample.kline"));
            assert!(pipelines[0].dag_json.contains("ema"));
            assert!(pipelines[0].dag_json.contains("sample-kline.emit"));

            let datasources = db::datasources::list(conn).unwrap();
            assert_eq!(datasources.len(), 1);
            assert_eq!(datasources[0].name, DEMO_DATASOURCE_NAME);
            assert_eq!(datasources[0].plugin, DEMO_PLUGIN_NAME);
            assert!(datasources[0].config.contains("\"url\":\"sample\""));
            assert!(datasources[0].config.contains("\"symbol\":\"BTC/USDT\""));
            assert!(datasources[0].config.contains("\"interval\":\"5min\""));

            let resources = db::applications::resources_for(conn, apps[0].id).unwrap();
            assert!(resources
                .iter()
                .any(|(k, id)| k == "pipeline" && id.as_deref() == Some(DEMO_PIPELINE_NAME)));
            assert!(resources
                .iter()
                .any(|(k, id)| k == "datasource" && id.as_deref() == Some(DEMO_DATASOURCE_NAME)));

            let audit_count = db::audit::count(conn).unwrap();
            assert!(audit_count >= 4);
            let events = db::audit::query(conn, Some(apps[0].id), 10).unwrap();
            assert!(events.iter().any(|e| e.action == "application.create"
                && e.resource_id.as_deref() == Some(DEMO_APP_NAME)));
            assert!(events.iter().any(|e| e.action == "pipeline.create"
                && e.resource_id.as_deref() == Some(DEMO_PIPELINE_NAME)));
            assert!(events.iter().any(|e| e.action == "datasource.create"
                && e.resource_id.as_deref() == Some(DEMO_DATASOURCE_NAME)));
            assert!(events.iter().any(|e| e.action == "demo.seed"));
        });
    }

    #[test]
    fn seed_demo_is_idempotent_when_all_three_resources_already_exist() {
        run(|conn| {
            let first = seed_demo(conn).unwrap();
            assert!(first.created);

            let audit_count_before = db::audit::count(conn).unwrap();
            let apps_before = db::applications::list(conn).unwrap().len();
            let pipelines_before = db::pipelines::list(conn).unwrap().len();
            let datasources_before = db::datasources::list(conn).unwrap().len();

            let second = seed_demo(conn).unwrap();
            assert!(!second.created);
            assert_eq!(second.application_id, None);
            assert_eq!(second.pipeline_id, None);
            assert_eq!(second.datasource_name, None);
            assert_eq!(second.audit_events, 0);

            let audit_count_after = db::audit::count(conn).unwrap();
            assert_eq!(audit_count_before, audit_count_after);
            assert_eq!(apps_before, db::applications::list(conn).unwrap().len());
            assert_eq!(
                pipelines_before,
                db::pipelines::list(conn).unwrap().len()
            );
            assert_eq!(
                datasources_before,
                db::datasources::list(conn).unwrap().len()
            );
        });
    }

    #[test]
    fn seed_demo_completes_partial_state_when_only_application_exists() {
        run(|conn| {
            let existing = db::applications::create(conn, DEMO_APP_NAME).unwrap();
            let report = seed_demo(conn).unwrap();
            assert!(report.created);
            assert_eq!(report.application_id, Some(existing.id));
            assert!(report.pipeline_id.is_some());
            assert_eq!(report.datasource_name.as_deref(), Some(DEMO_DATASOURCE_NAME));

            let apps = db::applications::list(conn).unwrap();
            assert_eq!(apps.len(), 1);
            assert_eq!(apps[0].id, existing.id);
        });
    }

    #[test]
    fn seed_demo_writes_all_inside_one_transaction() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        let conn = db.lock().unwrap();

        let report = seed_demo(&conn).unwrap();
        assert!(report.created);

        let apps = db::applications::list(&conn).unwrap();
        let pipelines = db::pipelines::list(&conn).unwrap();
        let datasources = db::datasources::list(&conn).unwrap();
        let audit_count = db::audit::count(&conn).unwrap();

        assert_eq!(apps.len(), 1);
        assert_eq!(pipelines.len(), 1);
        assert_eq!(datasources.len(), 1);
        assert!(audit_count >= 4);
    }

    #[test]
    fn seed_demo_through_db_helper_locks_and_runs() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();

        let report = seed_demo_db(&db).unwrap();
        assert!(report.created);

        let conn = db.lock().unwrap();
        assert_eq!(db::applications::list(&conn).unwrap().len(), 1);
        assert_eq!(db::pipelines::list(&conn).unwrap().len(), 1);
        assert_eq!(db::datasources::list(&conn).unwrap().len(), 1);
    }
}
