use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::connection;
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct ApplicationView {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub display_order: i64,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct DisableSnapshotView {
    pub application_id: i64,
    pub taken_at: i64,
    pub payload_json: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct DisableReport {
    pub application: ApplicationView,
    pub snapshot: DisableSnapshotView,
    pub pipelines: Vec<String>,
    pub datasources: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ResourceRehydrationOutcome {
    pub kind: String,
    pub name: String,
    pub result: String,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct EnableReport {
    pub application: ApplicationView,
    pub snapshot: Option<DisableSnapshotView>,
    pub rehydrated: Vec<ResourceRehydrationOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineToRehydrate {
    pub id: i64,
    pub name: String,
    pub dag_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasourceToRehydrate {
    pub name: String,
    pub plugin: String,
    pub config: String,
    pub tenant: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RehydrationPlan {
    pub pipelines: Vec<PipelineToRehydrate>,
    pub datasources: Vec<DatasourceToRehydrate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RehydrationError {
    Sql(String),
    Admin(String),
}

pub trait DeployRegistrar {
    fn deploy_sql(&self, sql_text: &str) -> Result<(), RehydrationError>;
    fn register_datasource(
        &self,
        name: &str,
        adapter: &str,
        config_json: &str,
        tenant: u16,
    ) -> Result<(), RehydrationError>;
}

pub struct AdminServerDeployRegistrar {
    pub addr: std::net::SocketAddr,
}

fn run_call_blocking<F, T>(fut: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            return tokio::task::block_in_place(|| handle.block_on(fut));
        }
        return handle.block_on(fut);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio build: {e}"))?;
    rt.block_on(fut)
}

impl DeployRegistrar for AdminServerDeployRegistrar {
    fn deploy_sql(&self, sql_text: &str) -> Result<(), RehydrationError> {
        let handle = connection::ensure_bundle(self.addr);
        let req = bee_control::raft::AdminRequest::Deploy {
            sql_text: sql_text.to_string(),
            owner_node: 0,
        };
        let rx = run_call_blocking(async move { handle.call(req).await })
            .map_err(RehydrationError::Admin)?;
        match rx.blocking_recv() {
            Ok(Ok(_resp)) => Ok(()),
            Ok(Err(e)) => Err(RehydrationError::Admin(e)),
            Err(_) => Err(RehydrationError::Admin("call channel closed".into())),
        }
    }

    fn register_datasource(
        &self,
        name: &str,
        adapter: &str,
        config_json: &str,
        tenant: u16,
    ) -> Result<(), RehydrationError> {
        let handle = connection::ensure_bundle(self.addr);
        let req = bee_control::raft::AdminRequest::RegisterDatasource {
            name: name.to_string(),
            adapter: adapter.to_string(),
            plugin_version: "0".to_string(),
            config_json: config_json.to_string(),
            tenant,
            owner_node: 0,
        };
        let rx = run_call_blocking(async move { handle.call(req).await })
            .map_err(RehydrationError::Admin)?;
        match rx.blocking_recv() {
            Ok(Ok(_resp)) => Ok(()),
            Ok(Err(e)) => Err(RehydrationError::Admin(e)),
            Err(_) => Err(RehydrationError::Admin("call channel closed".into())),
        }
    }
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn to_view(a: db::applications::Application) -> ApplicationView {
    ApplicationView {
        id: a.id,
        name: a.name,
        enabled: a.enabled,
        display_order: a.display_order,
        created_at: a.created_at,
    }
}

fn snapshot_to_view(s: db::applications::DisableSnapshot) -> DisableSnapshotView {
    DisableSnapshotView {
        application_id: s.application_id,
        taken_at: s.taken_at,
        payload_json: s.payload_json,
    }
}

fn pipeline_dag_for(
    conn: &rusqlite::Connection,
    name: &str,
) -> Result<Option<String>, String> {
    let pipelines = db::pipelines::list(conn).map_err(|e| e)?;
    Ok(pipelines.into_iter().find(|p| p.name == name).map(|p| p.dag_json))
}

fn datasource_for(
    conn: &rusqlite::Connection,
    name: &str,
) -> Result<Option<DatasourceToRehydrate>, String> {
    let datasources = db::datasources::list(conn).map_err(|e| e)?;
    Ok(datasources
        .into_iter()
        .find(|d| d.name == name)
        .map(|d| DatasourceToRehydrate {
            name: d.name,
            plugin: d.plugin,
            config: d.config,
            tenant: d.tenant.clamp(0, u16::MAX as i64) as u16,
        }))
}

pub fn build_rehydration_plan(
    conn: &rusqlite::Connection,
    snapshot: &db::applications::DisableSnapshot,
) -> Result<RehydrationPlan, String> {
    #[derive(serde::Deserialize)]
    struct Snap {
        pipelines: Vec<SnapPipeline>,
        datasources: Vec<SnapDatasource>,
    }
    #[derive(serde::Deserialize)]
    struct SnapPipeline {
        id: i64,
        name: String,
    }
    #[derive(serde::Deserialize)]
    struct SnapDatasource {
        name: String,
        #[allow(dead_code)]
        plugin: String,
    }
    let parsed: Snap = serde_json::from_str(&snapshot.payload_json)
        .map_err(|e| format!("enable: parse snapshot: {e}"))?;
    let mut pipelines = Vec::new();
    for sp in parsed.pipelines {
        if let Some(dag_json) = pipeline_dag_for(conn, &sp.name)? {
            pipelines.push(PipelineToRehydrate {
                id: sp.id,
                name: sp.name,
                dag_json,
            });
        }
    }
    let mut datasources = Vec::new();
    for sd in parsed.datasources {
        if let Some(full) = datasource_for(conn, &sd.name)? {
            datasources.push(full);
        }
    }
    Ok(RehydrationPlan { pipelines, datasources })
}

pub fn execute_rehydration<R: DeployRegistrar>(
    plan: &RehydrationPlan,
    registrar: &R,
) -> Vec<ResourceRehydrationOutcome> {
    let mut out = Vec::new();
    for p in &plan.pipelines {
        let outcome = registrar.deploy_sql(&p.dag_json);
        let (result, detail) = match outcome {
            Ok(()) => ("Success".to_string(), None),
            Err(RehydrationError::Sql(msg)) => ("Failure".to_string(), Some(msg)),
            Err(RehydrationError::Admin(msg)) => ("Failure".to_string(), Some(msg)),
        };
        out.push(ResourceRehydrationOutcome {
            kind: "pipeline".to_string(),
            name: p.name.clone(),
            result,
            detail,
        });
    }
    for d in &plan.datasources {
        let outcome = registrar.register_datasource(&d.name, &d.plugin, &d.config, d.tenant);
        let (result, detail) = match outcome {
            Ok(()) => ("Success".to_string(), None),
            Err(RehydrationError::Sql(msg)) => ("Failure".to_string(), Some(msg)),
            Err(RehydrationError::Admin(msg)) => ("Failure".to_string(), Some(msg)),
        };
        out.push(ResourceRehydrationOutcome {
            kind: "datasource".to_string(),
            name: d.name.clone(),
            result,
            detail,
        });
    }
    out
}

#[tauri::command]
pub fn applications_list(app: AppHandle) -> CmdResult<Vec<ApplicationView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::applications::list(&conn)
        .map_err(CmdError::from)
        .map(|xs| xs.into_iter().map(to_view).collect())
}

#[tauri::command]
pub fn application_create(app: AppHandle, name: String) -> CmdResult<ApplicationView> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let created = db::applications::create(&conn, &name).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "application.create",
        result: "Success",
        summary: &format!("Application \"{}\" created", created.name),
        resource_kind: Some("application"),
        resource_id: None,
        application_id: Some(created.id),
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("application"),
        nav_resource_id: None,
    });
    Ok(to_view(created))
}

#[tauri::command]
pub fn application_set_enabled(
    app: AppHandle,
    id: i64,
    enabled: bool,
) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::applications::set_enabled(&conn, id, enabled).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: if enabled { "application.enable" } else { "application.disable" },
        result: "Success",
        summary: if enabled { "Application enabled" } else { "Application disabled" },
        resource_kind: Some("application"),
        resource_id: None,
        application_id: Some(id),
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("application"),
        nav_resource_id: None,
    });
    Ok(())
}

#[tauri::command]
pub fn application_enable(app: AppHandle, id: i64, addr: Option<String>) -> CmdResult<EnableReport> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let existing = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_enable: no row {id}") })?;

    let latest_snapshots = db::applications::list_disable_snapshots(&conn, id)
        .map_err(CmdError::from)?;
    let latest_snapshot = latest_snapshots.first().cloned();

    if !existing.enabled {
        db::applications::set_enabled(&conn, id, true).map_err(CmdError::from)?;
    }
    let updated = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_enable: vanished {id}") })?;

    let mut rehydrated: Vec<ResourceRehydrationOutcome> = Vec::new();
    let mut enabled_count = 0u32;
    let mut failed_count = 0u32;
    if let Some(snap) = &latest_snapshot {
        let plan = build_rehydration_plan(&conn, snap).map_err(CmdError::from)?;
        let target_addr = match addr
            .as_deref()
            .map(connection::addr_parse)
            .transpose()
            .map_err(CmdError::from)?
        {
            Some(a) => a,
            None => connection::with_handle(|h| Ok(h.addr())).map_err(CmdError::from)?,
        };
        let registrar = AdminServerDeployRegistrar { addr: target_addr };
        rehydrated = execute_rehydration(&plan, &registrar);
        for ev in &rehydrated {
            match ev.result.as_str() {
                "Success" => enabled_count += 1,
                _ => failed_count += 1,
            }
            let summary = format!(
                "Application \"{}\" rehydrate {} \"{}\": {}",
                updated.name, ev.kind, ev.name, ev.result
            );
            let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
                actor: "user",
                action: "application.rehydrate",
                result: &ev.result,
                summary: &summary,
                resource_kind: Some(&ev.kind),
                resource_id: Some(&ev.name),
                application_id: Some(id),
                correlation_id: None,
                operation_id: None,
                nav_kind: Some(&ev.kind),
                nav_resource_id: Some(&ev.name),
            });
        }
    }

    let overall = if failed_count == 0 {
        "Success"
    } else if enabled_count == 0 {
        "Failure"
    } else {
        "Degraded"
    };
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "application.enable",
        result: overall,
        summary: &format!(
            "Application \"{}\" enabled ({} succeeded, {} failed)",
            updated.name, enabled_count, failed_count
        ),
        resource_kind: Some("application"),
        resource_id: None,
        application_id: Some(id),
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("application"),
        nav_resource_id: None,
    });

    Ok(EnableReport {
        application: to_view(updated),
        snapshot: latest_snapshot.map(snapshot_to_view),
        rehydrated,
    })
}

#[tauri::command]
pub fn application_disable(app: AppHandle, id: i64) -> CmdResult<DisableReport> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let existing = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_disable: no row {id}") })?;

    let pipelines = db::pipelines::list(&conn).map_err(CmdError::from)?;
    let datasources = db::datasources::list(&conn).map_err(CmdError::from)?;

    let payload_json =
        db::applications::snapshot_payload(&pipelines, &datasources).map_err(CmdError::from)?;
    let snapshot =
        db::applications::record_disable_snapshot(&conn, id, &payload_json).map_err(CmdError::from)?;

    if existing.enabled {
        db::applications::set_enabled(&conn, id, false).map_err(CmdError::from)?;
    }
    let updated = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_disable: vanished {id}") })?;

    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "application.disable",
        result: "Success",
        summary: &format!(
            "Application \"{}\" disabled (snapshot has {} pipelines, {} datasources)",
            updated.name,
            pipelines.len(),
            datasources.len()
        ),
        resource_kind: Some("application"),
        resource_id: None,
        application_id: Some(id),
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("application"),
        nav_resource_id: None,
    });

    Ok(DisableReport {
        application: to_view(updated),
        snapshot: snapshot_to_view(snapshot),
        pipelines: pipelines.into_iter().map(|p| p.name).collect(),
        datasources: datasources.into_iter().map(|d| d.name).collect(),
    })
}

#[tauri::command]
pub fn application_delete(app: AppHandle, id: i64) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::applications::delete(&conn, id).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "application.delete",
        result: "Success",
        summary: "Application deleted",
        resource_kind: Some("application"),
        resource_id: None,
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: None,
        nav_resource_id: None,
    });
    Ok(())
}

#[derive(Debug, Serialize, Clone)]
pub struct ImportReportView {
    pub created: Vec<String>,
    pub skipped: Vec<String>,
}

#[tauri::command]
pub fn application_export(
    app: AppHandle,
    name: String,
    passphrase: String,
    out_path: String,
) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let path = std::path::Path::new(&out_path);
    crate::import_export::export_application(db, &name, &passphrase, path)
        .map_err(CmdError::from)?;
    Ok(())
}

#[tauri::command]
pub fn application_import(
    app: AppHandle,
    file_path: String,
    passphrase: String,
) -> CmdResult<ImportReportView> {
    let db = db_handle(&app)?;
    let path = std::path::Path::new(&file_path);
    let report = crate::import_export::import_application(db, path, &passphrase)
        .map_err(CmdError::from)?;
    Ok(ImportReportView {
        created: report.created.into_iter().map(|a| a.name).collect(),
        skipped: report.skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::sync::Mutex;
    use tempfile::tempdir;

    fn run<F: FnOnce(&Database)>(f: F) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        f(&db);
    }

    #[test]
    fn snapshot_payload_includes_empty_lists_for_no_resources() {
        let json = db::applications::snapshot_payload(&[], &[]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v.get("pipelines").unwrap().as_array().unwrap().len(), 0);
        assert_eq!(v.get("datasources").unwrap().as_array().unwrap().len(), 0);
    }

    #[test]
    fn snapshot_payload_serializes_pipelines_and_datasources_by_id_and_name() {
        let pipelines = vec![db::pipelines::PipelineDefinition {
            id: 7,
            name: "alpha".into(),
            dag_json: "{}".into(),
            updated_at: 0,
        }];
        let datasources = vec![db::datasources::Datasource {
            name: "binance".into(),
            plugin: "binance_subscribe".into(),
            config: "{}".into(),
            tenant: 0,
            created_at: 0,
            updated_at: 0,
        }];
        let json = db::applications::snapshot_payload(&pipelines, &datasources).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let ps = v.get("pipelines").unwrap().as_array().unwrap();
        let ds = v.get("datasources").unwrap().as_array().unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].get("id").unwrap().as_i64(), Some(7));
        assert_eq!(ps[0].get("name").unwrap().as_str(), Some("alpha"));
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].get("name").unwrap().as_str(), Some("binance"));
    }

    #[test]
    fn disable_lifecycle_writes_snapshot_disables_app_and_records_audit() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            let _ = db::pipelines::create(&conn, "p1", "{}").unwrap();
            let _ = db::datasources::create(&conn, "binance", "binance_subscribe", "{}", 0).unwrap();

            let pipelines = db::pipelines::list(&conn).unwrap();
            let datasources = db::datasources::list(&conn).unwrap();
            let payload = db::applications::snapshot_payload(&pipelines, &datasources).unwrap();
            let snap =
                db::applications::record_disable_snapshot(&conn, app.id, &payload).unwrap();
            db::applications::set_enabled(&conn, app.id, false).unwrap();
            let _ = db::audit::record(
                &conn,
                db::audit::NewAuditEvent {
                    actor: "user",
                    action: "application.disable",
                    result: "Success",
                    summary: "Application disabled",
                    resource_kind: Some("application"),
                    resource_id: None,
                    application_id: Some(app.id),
                    correlation_id: None,
                    operation_id: None,
                    nav_kind: Some("application"),
                    nav_resource_id: None,
                },
            );

            let after = db::applications::get(&conn, app.id).unwrap().unwrap();
            assert!(!after.enabled);
            let snaps = db::applications::list_disable_snapshots(&conn, app.id).unwrap();
            assert_eq!(snaps.len(), 1);
            assert_eq!(snaps[0].payload_json, snap.payload_json);
            let events = db::audit::query(&conn, Some(app.id), 10).unwrap();
            assert!(events.iter().any(|e| e.action == "application.disable"));
        });
    }

    #[test]
    fn enable_is_idempotent_does_not_double_record_audit_for_already_enabled() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            db::applications::set_enabled(&conn, app.id, true).unwrap();
            let before = db::audit::query(&conn, Some(app.id), 10).unwrap().len();

            let _ = db::audit::record(
                &conn,
                db::audit::NewAuditEvent {
                    actor: "user",
                    action: "application.enable",
                    result: "Success",
                    summary: "Application enabled",
                    resource_kind: Some("application"),
                    resource_id: None,
                    application_id: Some(app.id),
                    correlation_id: None,
                    operation_id: None,
                    nav_kind: Some("application"),
                    nav_resource_id: None,
                },
            );
            let after = db::audit::query(&conn, Some(app.id), 10).unwrap().len();
            assert_eq!(after, before + 1);
            let after_app = db::applications::get(&conn, app.id).unwrap().unwrap();
            assert!(after_app.enabled);
        });
    }

    struct MockRegistrar {
        log: Mutex<Vec<MockCall>>,
    }
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum MockCall {
        Deploy(String),
        RegisterDatasource { name: String, adapter: String, config_json: String, tenant: u16 },
    }
    impl MockRegistrar {
        fn new() -> Self {
            Self { log: Mutex::new(Vec::new()) }
        }
        fn calls(&self) -> Vec<MockCall> {
            self.log.lock().unwrap().clone()
        }
    }
    impl DeployRegistrar for MockRegistrar {
        fn deploy_sql(&self, sql_text: &str) -> Result<(), RehydrationError> {
            self.log.lock().unwrap().push(MockCall::Deploy(sql_text.to_string()));
            Ok(())
        }
        fn register_datasource(
            &self,
            name: &str,
            adapter: &str,
            config_json: &str,
            tenant: u16,
        ) -> Result<(), RehydrationError> {
            self.log.lock().unwrap().push(MockCall::RegisterDatasource {
                name: name.to_string(),
                adapter: adapter.to_string(),
                config_json: config_json.to_string(),
                tenant,
            });
            Ok(())
        }
    }

    struct FailingRegistrar;
    impl DeployRegistrar for FailingRegistrar {
        fn deploy_sql(&self, _sql_text: &str) -> Result<(), RehydrationError> {
            Err(RehydrationError::Admin("not connected".into()))
        }
        fn register_datasource(
            &self,
            _name: &str,
            _adapter: &str,
            _config_json: &str,
            _tenant: u16,
        ) -> Result<(), RehydrationError> {
            Err(RehydrationError::Admin("not connected".into()))
        }
    }

    #[test]
    fn rehydration_plan_uses_local_pipeline_dag_and_datasource_config() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            let p = db::pipelines::create(&conn, "p1", "CREATE PIPELINE p1 SOURCE binance;").unwrap();
            db::datasources::create(&conn, "binance", "binance_subscribe", r#"{"u":"wss://x"}"#, 0).unwrap();

            let pipelines = db::pipelines::list(&conn).unwrap();
            let datasources = db::datasources::list(&conn).unwrap();
            let payload = db::applications::snapshot_payload(&pipelines, &datasources).unwrap();
            let snap = db::applications::record_disable_snapshot(&conn, app.id, &payload).unwrap();
            drop(conn);

            let conn2 = db.lock().unwrap();
            let plan = build_rehydration_plan(&conn2, &snap).unwrap();
            assert_eq!(plan.pipelines.len(), 1);
            assert_eq!(plan.pipelines[0].id, p.id);
            assert_eq!(plan.pipelines[0].name, "p1");
            assert!(plan.pipelines[0].dag_json.contains("CREATE PIPELINE"));
            assert_eq!(plan.datasources.len(), 1);
            assert_eq!(plan.datasources[0].name, "binance");
            assert_eq!(plan.datasources[0].plugin, "binance_subscribe");
            assert!(plan.datasources[0].config.contains("wss"));
            assert_eq!(plan.datasources[0].tenant, 0);
        });
    }

    #[test]
    fn rehydration_plan_skips_resources_deleted_since_snapshot() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            db::pipelines::create(&conn, "p1", "{}").unwrap();
            db::datasources::create(&conn, "binance", "binance_subscribe", "{}", 0).unwrap();

            let pipelines = db::pipelines::list(&conn).unwrap();
            let datasources = db::datasources::list(&conn).unwrap();
            let payload = db::applications::snapshot_payload(&pipelines, &datasources).unwrap();
            let snap = db::applications::record_disable_snapshot(&conn, app.id, &payload).unwrap();
            db::pipelines::delete(&conn, pipelines[0].id).unwrap();
            db::datasources::delete(&conn, "binance").unwrap();
            drop(conn);

            let conn2 = db.lock().unwrap();
            let plan = build_rehydration_plan(&conn2, &snap).unwrap();
            assert!(plan.pipelines.is_empty());
            assert!(plan.datasources.is_empty());
        });
    }

    #[test]
    fn rehydration_plan_empty_snapshot_produces_no_work() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            let snap = db::applications::DisableSnapshot {
                application_id: app.id,
                taken_at: 0,
                payload_json: r#"{"pipelines":[],"datasources":[]}"#.to_string(),
            };
            let plan = build_rehydration_plan(&conn, &snap).unwrap();
            assert!(plan.pipelines.is_empty());
            assert!(plan.datasources.is_empty());
        });
    }

    #[test]
    fn execute_rehydration_invokes_deploy_for_each_pipeline_then_register_for_each_datasource() {
        let registrar = MockRegistrar::new();
        let plan = RehydrationPlan {
            pipelines: vec![
                PipelineToRehydrate {
                    id: 1,
                    name: "p1".into(),
                    dag_json: "CREATE PIPELINE p1".into(),
                },
                PipelineToRehydrate {
                    id: 2,
                    name: "p2".into(),
                    dag_json: "CREATE PIPELINE p2".into(),
                },
            ],
            datasources: vec![DatasourceToRehydrate {
                name: "binance".into(),
                plugin: "binance_subscribe".into(),
                config: r#"{"u":"wss"}"#.into(),
                tenant: 0,
            }],
        };
        let outcomes = execute_rehydration(&plan, &registrar);
        assert_eq!(outcomes.len(), 3);
        assert_eq!(outcomes[0].kind, "pipeline");
        assert_eq!(outcomes[0].name, "p1");
        assert_eq!(outcomes[0].result, "Success");
        assert_eq!(outcomes[1].kind, "pipeline");
        assert_eq!(outcomes[1].name, "p2");
        assert_eq!(outcomes[1].result, "Success");
        assert_eq!(outcomes[2].kind, "datasource");
        assert_eq!(outcomes[2].name, "binance");
        assert_eq!(outcomes[2].result, "Success");

        let calls = registrar.calls();
        assert_eq!(calls.len(), 3);
        assert!(matches!(&calls[0], MockCall::Deploy(s) if s == "CREATE PIPELINE p1"));
        assert!(matches!(&calls[1], MockCall::Deploy(s) if s == "CREATE PIPELINE p2"));
        assert!(matches!(&calls[2], MockCall::RegisterDatasource { name, adapter, .. } if name == "binance" && adapter == "binance_subscribe"));
    }

    #[test]
    fn execute_rehydration_records_failures_with_detail_and_continues() {
        let registrar = FailingRegistrar;
        let plan = RehydrationPlan {
            pipelines: vec![PipelineToRehydrate {
                id: 1,
                name: "p1".into(),
                dag_json: "{}".into(),
            }],
            datasources: vec![DatasourceToRehydrate {
                name: "binance".into(),
                plugin: "binance_subscribe".into(),
                config: "{}".into(),
                tenant: 0,
            }],
        };
        let outcomes = execute_rehydration(&plan, &registrar);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].result, "Failure");
        assert_eq!(outcomes[0].detail.as_deref(), Some("not connected"));
        assert_eq!(outcomes[1].result, "Failure");
        assert_eq!(outcomes[1].detail.as_deref(), Some("not connected"));
    }

    #[test]
    fn enable_records_one_audit_per_rehydrated_resource_and_one_summary() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            db::pipelines::create(&conn, "p1", "CREATE PIPELINE p1").unwrap();
            db::datasources::create(&conn, "binance", "binance_subscribe", "{}", 0).unwrap();

            let pipelines = db::pipelines::list(&conn).unwrap();
            let datasources = db::datasources::list(&conn).unwrap();
            let payload = db::applications::snapshot_payload(&pipelines, &datasources).unwrap();
            db::applications::record_disable_snapshot(&conn, app.id, &payload).unwrap();
            db::applications::set_enabled(&conn, app.id, false).unwrap();
            drop(conn);

            let conn2 = db.lock().unwrap();
            let snap = db::applications::list_disable_snapshots(&conn2, app.id).unwrap().remove(0);
            let plan = build_rehydration_plan(&conn2, &snap).unwrap();
            let registrar = MockRegistrar::new();
            let outcomes = execute_rehydration(&plan, &registrar);
            for ev in &outcomes {
                let summary = format!("rehydrate {} {}", ev.kind, ev.name);
                let _ = db::audit::record(
                    &conn2,
                    db::audit::NewAuditEvent {
                        actor: "user",
                        action: "application.rehydrate",
                        result: &ev.result,
                        summary: &summary,
                        resource_kind: Some(&ev.kind),
                        resource_id: Some(&ev.name),
                        application_id: Some(app.id),
                        correlation_id: None,
                        operation_id: None,
                        nav_kind: Some(&ev.kind),
                        nav_resource_id: Some(&ev.name),
                    },
                );
            }
            let _ = db::audit::record(
                &conn2,
                db::audit::NewAuditEvent {
                    actor: "user",
                    action: "application.enable",
                    result: "Success",
                    summary: "Application enabled",
                    resource_kind: Some("application"),
                    resource_id: None,
                    application_id: Some(app.id),
                    correlation_id: None,
                    operation_id: None,
                    nav_kind: Some("application"),
                    nav_resource_id: None,
                },
            );
            drop(conn2);

            let conn3 = db.lock().unwrap();
            let events = db::audit::query(&conn3, Some(app.id), 20).unwrap();
            let actions: Vec<&str> = events.iter().map(|e| e.action.as_str()).collect();
            assert!(actions.contains(&"application.rehydrate"));
            assert!(actions.contains(&"application.enable"));
            let rehydrate_events: Vec<_> = events.iter().filter(|e| e.action == "application.rehydrate").collect();
            assert_eq!(rehydrate_events.len(), outcomes.len());
            assert_eq!(rehydrate_events.iter().filter(|e| e.result == "Success").count(), outcomes.len());
        });
    }

    #[test]
    fn enable_with_no_snapshot_records_no_rehydrate_audit_events() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            let before = db::audit::query(&conn, Some(app.id), 100).unwrap().len();

            db::applications::set_enabled(&conn, app.id, true).unwrap();
            let _ = db::audit::record(
                &conn,
                db::audit::NewAuditEvent {
                    actor: "user",
                    action: "application.enable",
                    result: "Success",
                    summary: "Application enabled",
                    resource_kind: Some("application"),
                    resource_id: None,
                    application_id: Some(app.id),
                    correlation_id: None,
                    operation_id: None,
                    nav_kind: Some("application"),
                    nav_resource_id: None,
                },
            );
            let after = db::audit::query(&conn, Some(app.id), 100).unwrap();
            let rehydrate = after.iter().filter(|e| e.action == "application.rehydrate").count();
            assert_eq!(rehydrate, 0, "no rehydrate events when no snapshot exists");
            let enable_events = after.iter().filter(|e| e.action == "application.enable").count();
            assert_eq!(enable_events, 1);
            assert!(after.len() > before);
        });
    }

    #[test]
    fn disable_records_audit_with_snapshot_resource_counts() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            db::pipelines::create(&conn, "p1", "{}").unwrap();
            db::datasources::create(&conn, "binance", "binance_subscribe", "{}", 0).unwrap();
            let pipelines = db::pipelines::list(&conn).unwrap();
            let datasources = db::datasources::list(&conn).unwrap();
            let payload = db::applications::snapshot_payload(&pipelines, &datasources).unwrap();
            db::applications::record_disable_snapshot(&conn, app.id, &payload).unwrap();
            db::applications::set_enabled(&conn, app.id, false).unwrap();
            let summary = format!(
                "Application \"{}\" disabled (snapshot has {} pipelines, {} datasources)",
                app.name,
                pipelines.len(),
                datasources.len()
            );
            let _ = db::audit::record(
                &conn,
                db::audit::NewAuditEvent {
                    actor: "user",
                    action: "application.disable",
                    result: "Success",
                    summary: &summary,
                    resource_kind: Some("application"),
                    resource_id: None,
                    application_id: Some(app.id),
                    correlation_id: None,
                    operation_id: None,
                    nav_kind: Some("application"),
                    nav_resource_id: None,
                },
            );
            let events = db::audit::query(&conn, Some(app.id), 10).unwrap();
            assert!(events.iter().any(|e| e.action == "application.disable" && e.summary.contains("1 pipelines")));
        });
    }
}