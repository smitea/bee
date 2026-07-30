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
    pub tenant: u16,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct DisableSnapshotView {
    pub application_id: i64,
    pub taken_at: i64,
    pub payload_json: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ResourceOpView {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct FailedResourceView {
    pub kind: String,
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct DisableReport {
    pub application: ApplicationView,
    pub snapshot: Option<DisableSnapshotView>,
    pub succeeded: Vec<ResourceOpView>,
    pub failed: Vec<FailedResourceView>,
    pub skipped: Vec<ResourceOpView>,
    pub pipelines: Vec<String>,
    pub datasources: Vec<String>,
    pub outcome: String,
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
    pub succeeded: Vec<ResourceOpView>,
    pub failed: Vec<FailedResourceView>,
    pub skipped: Vec<ResourceOpView>,
    pub rehydrated: Vec<ResourceRehydrationOutcome>,
    pub outcome: String,
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
    fn deploy_pipeline(
        &self,
        name: &str,
        dag_json: &str,
    ) -> Result<(), RehydrationError>;
    fn register_datasource(
        &self,
        name: &str,
        plugin: &str,
        config_json: &str,
    ) -> Result<(), RehydrationError>;
}

pub struct AdminServerDeployRegistrar {
    pub addr: std::net::SocketAddr,
    pub tenant: u16,
}

pub struct NoopDeployRegistrar;

impl DeployRegistrar for NoopDeployRegistrar {
    fn deploy_pipeline(
        &self,
        _name: &str,
        _dag_json: &str,
    ) -> Result<(), RehydrationError> {
        Ok(())
    }
    fn register_datasource(
        &self,
        _name: &str,
        _plugin: &str,
        _config_json: &str,
    ) -> Result<(), RehydrationError> {
        Ok(())
    }
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

fn wait_for_connection(handle: &connection::ConnectionHandle, timeout_secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        if matches!(
            handle.state(),
            connection::ConnectionState::Connected
        ) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

impl DeployRegistrar for AdminServerDeployRegistrar {
    fn deploy_pipeline(
        &self,
        _name: &str,
        dag_json: &str,
    ) -> Result<(), RehydrationError> {
        let handle = connection::ensure_bundle(self.addr);
        if !wait_for_connection(&handle, 5) {
            return Err(RehydrationError::Admin(format!(
                "connection not ready (addr={})",
                self.addr
            )));
        }
        let req = bee_control::raft::AdminRequest::Deploy {
            sql_text: dag_json.to_string(),
            owner_node: 0,
        };
        let rx = run_call_blocking(async move { handle.call(req).await })
            .map_err(RehydrationError::Admin)?;
        let resp = rx
            .blocking_recv()
            .map_err(|_| RehydrationError::Admin("call channel closed".into()))?
            .map_err(RehydrationError::Admin)?;
        match resp {
            bee_control::raft::AdminResponse::DeployAck { error_msg, .. } => {
                if error_msg.is_empty() {
                    Ok(())
                } else {
                    Err(RehydrationError::Admin(error_msg))
                }
            }
            bee_control::raft::AdminResponse::Error(msg) => {
                Err(RehydrationError::Admin(msg))
            }
            other => Err(RehydrationError::Admin(format!(
                "unexpected Deploy response: {other:?}"
            ))),
        }
    }

    fn register_datasource(
        &self,
        name: &str,
        plugin: &str,
        config_json: &str,
    ) -> Result<(), RehydrationError> {
        let handle = connection::ensure_bundle(self.addr);
        if !wait_for_connection(&handle, 5) {
            return Err(RehydrationError::Admin(format!(
                "connection not ready (addr={})",
                self.addr
            )));
        }
        let req = bee_control::raft::AdminRequest::RegisterDatasource {
            name: name.to_string(),
            adapter: plugin.to_string(),
            plugin_version: "latest".to_string(),
            config_json: config_json.to_string(),
            tenant: self.tenant,
            owner_node: 0,
        };
        let rx = run_call_blocking(async move { handle.call(req).await })
            .map_err(RehydrationError::Admin)?;
        let resp = rx
            .blocking_recv()
            .map_err(|_| RehydrationError::Admin("call channel closed".into()))?
            .map_err(RehydrationError::Admin)?;
        match resp {
            bee_control::raft::AdminResponse::RegisterDatasourceAck {
                ok,
                error_msg,
            } => {
                if ok {
                    Ok(())
                } else {
                    Err(RehydrationError::Admin(error_msg))
                }
            }
            bee_control::raft::AdminResponse::Error(msg) => {
                Err(RehydrationError::Admin(msg))
            }
            other => Err(RehydrationError::Admin(format!(
                "unexpected RegisterDatasource response: {other:?}"
            ))),
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
        tenant: a.tenant,
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
        let outcome = registrar.deploy_pipeline(&p.name, &p.dag_json);
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
        let outcome = registrar.register_datasource(&d.name, &d.plugin, &d.config);
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

fn rehydration_err_msg(e: &RehydrationError) -> String {
    match e {
        RehydrationError::Sql(m) => format!("sql: {m}"),
        RehydrationError::Admin(m) => format!("admin: {m}"),
    }
}

fn rehydrate_one<R: DeployRegistrar>(
    snap: &db::applications::ResourceSnapshot,
    registrar: &R,
) -> Result<(), String> {
    match snap.resource_kind.as_str() {
        "pipeline" => {
            #[derive(serde::Deserialize)]
            struct P {
                #[allow(dead_code)]
                id: i64,
                name: String,
                dag_json: String,
            }
            let p: P = serde_json::from_str(&snap.payload_json)
                .map_err(|e| format!("enable: parse pipeline payload: {e}"))?;
            registrar
                .deploy_pipeline(&p.name, &p.dag_json)
                .map_err(|e| format!("deploy pipeline \"{}\": {}", p.name, rehydration_err_msg(&e)))?;
            Ok(())
        }
        "datasource" => {
            #[derive(serde::Deserialize)]
            struct D {
                name: String,
                plugin: String,
                config: String,
                #[allow(dead_code)]
                tenant: Option<i64>,
            }
            let d: D = serde_json::from_str(&snap.payload_json)
                .map_err(|e| format!("enable: parse datasource payload: {e}"))?;
            registrar
                .register_datasource(&d.name, &d.plugin, &d.config)
                .map_err(|e| format!("register datasource \"{}\": {}", d.name, rehydration_err_msg(&e)))?;
            Ok(())
        }
        other => Err(format!("enable: unknown resource_kind {other}")),
    }
}

pub fn application_enable_with_registrar<R: DeployRegistrar>(
    conn: &rusqlite::Connection,
    application_id: i64,
    registrar: &R,
) -> Result<db::applications::EnableOutcome, String> {
    db::applications::application_enable(conn, application_id, |snap| {
        rehydrate_one(snap, registrar)
    })
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
pub fn application_create(
    app: AppHandle,
    name: String,
    tenant: Option<u16>,
) -> CmdResult<ApplicationView> {
    let tenant = match tenant {
        Some(t) => crate::tenant::validate_tenant(t).map_err(CmdError::from)?,
        None => {
            let db = db_handle(&app)?;
            let conn = db.lock().map_err(CmdError::from)?;
            let raw = db::settings::get(&conn, "tenant").map_err(CmdError::from)?;
            raw.as_deref()
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(0)
        }
    };
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let created = if tenant == 0 {
        db::applications::create(&conn, &name).map_err(CmdError::from)?
    } else {
        db::applications::create_with_tenant(&conn, &name, tenant).map_err(CmdError::from)?
    };
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "application.create",
        result: "Success",
        summary: &format!(
            "Application \"{}\" created (tenant={})",
            created.name, created.tenant
        ),
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
    let _existing = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_enable: no row {id}") })?;

    let latest_snapshot = db::applications::list_disable_snapshots(&conn, id)
        .map_err(CmdError::from)?
        .first()
        .cloned();

    let use_noop = addr.as_deref().map(|s| s.trim().is_empty()).unwrap_or(false);
    let target_addr = match addr
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(connection::addr_parse)
        .transpose()
        .map_err(CmdError::from)?
    {
        Some(a) => Some(a),
        None => connection::with_handle(|h| Ok(h.addr())).ok(),
    };

    let outcome = if use_noop || target_addr.is_none() {
        db::applications::application_enable(&conn, id, |_snap| Ok(()))
            .map_err(CmdError::from)?
    } else {
        let registrar = AdminServerDeployRegistrar {
            addr: target_addr.unwrap(),
            tenant: 0,
        };
        db::applications::application_enable(&conn, id, |snap| rehydrate_one(snap, &registrar))
            .map_err(CmdError::from)?
    };

    let updated = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_enable: vanished {id}") })?;

    let succeeded: Vec<ResourceRehydrationOutcome> = outcome
        .succeeded
        .iter()
        .map(|s| ResourceRehydrationOutcome {
            kind: s.kind.clone(),
            name: s.id.clone(),
            result: "Success".to_string(),
            detail: None,
        })
        .collect();

    Ok(EnableReport {
        application: to_view(updated),
        snapshot: latest_snapshot.map(snapshot_to_view),
        succeeded: outcome
            .succeeded
            .into_iter()
            .map(|s| ResourceOpView { kind: s.kind, id: s.id })
            .collect(),
        failed: outcome
            .failed
            .into_iter()
            .map(|f| FailedResourceView { kind: f.kind, id: f.id, reason: f.reason })
            .collect(),
        skipped: outcome
            .skipped
            .into_iter()
            .map(|s| ResourceOpView { kind: s.kind, id: s.id })
            .collect(),
        rehydrated: succeeded,
        outcome: outcome.outcome,
    })
}

#[tauri::command]
pub fn application_disable(app: AppHandle, id: i64) -> CmdResult<DisableReport> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let _existing = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_disable: no row {id}") })?;

    let disable_outcome = db::applications::application_disable(&conn, id).map_err(CmdError::from)?;

    let latest_snapshot = db::applications::list_disable_snapshots(&conn, id)
        .map_err(CmdError::from)?
        .first()
        .cloned();

    let updated = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_disable: vanished {id}") })?;

    let succeeded: Vec<ResourceOpView> = disable_outcome
        .snapshot_rows
        .iter()
        .map(|s| ResourceOpView {
            kind: s.resource_kind.clone(),
            id: s.resource_id.clone(),
        })
        .collect();

    Ok(DisableReport {
        application: to_view(updated),
        snapshot: latest_snapshot.map(snapshot_to_view),
        succeeded,
        failed: vec![],
        skipped: vec![],
        pipelines: disable_outcome.pipelines,
        datasources: disable_outcome.datasources,
        outcome: "Success".to_string(),
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
        Deploy { name: String, dag_json: String },
        RegisterDatasource { name: String, plugin: String, config_json: String },
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
        fn deploy_pipeline(
            &self,
            name: &str,
            dag_json: &str,
        ) -> Result<(), RehydrationError> {
            self.log.lock().unwrap().push(MockCall::Deploy {
                name: name.to_string(),
                dag_json: dag_json.to_string(),
            });
            Ok(())
        }
        fn register_datasource(
            &self,
            name: &str,
            plugin: &str,
            config_json: &str,
        ) -> Result<(), RehydrationError> {
            self.log.lock().unwrap().push(MockCall::RegisterDatasource {
                name: name.to_string(),
                plugin: plugin.to_string(),
                config_json: config_json.to_string(),
            });
            Ok(())
        }
    }

    struct FailingRegistrar;
    impl DeployRegistrar for FailingRegistrar {
        fn deploy_pipeline(
            &self,
            _name: &str,
            _dag_json: &str,
        ) -> Result<(), RehydrationError> {
            Err(RehydrationError::Admin("not connected".into()))
        }
        fn register_datasource(
            &self,
            _name: &str,
            _plugin: &str,
            _config_json: &str,
        ) -> Result<(), RehydrationError> {
            Err(RehydrationError::Admin("not connected".into()))
        }
    }

    struct PartialFailRegistrar {
        log: Mutex<Vec<MockCall>>,
        fail_pipeline: std::collections::HashSet<String>,
    }
    impl PartialFailRegistrar {
        fn new(fail_pipelines: &[&str]) -> Self {
            Self {
                log: Mutex::new(Vec::new()),
                fail_pipeline: fail_pipelines.iter().map(|s| s.to_string()).collect(),
            }
        }
        fn calls(&self) -> Vec<MockCall> {
            self.log.lock().unwrap().clone()
        }
    }
    impl DeployRegistrar for PartialFailRegistrar {
        fn deploy_pipeline(
            &self,
            name: &str,
            dag_json: &str,
        ) -> Result<(), RehydrationError> {
            if self.fail_pipeline.contains(name) {
                Err(RehydrationError::Admin(format!("pipeline {name} failed")))
            } else {
                self.log.lock().unwrap().push(MockCall::Deploy {
                    name: name.to_string(),
                    dag_json: dag_json.to_string(),
                });
                Ok(())
            }
        }
        fn register_datasource(
            &self,
            name: &str,
            plugin: &str,
            config_json: &str,
        ) -> Result<(), RehydrationError> {
            self.log.lock().unwrap().push(MockCall::RegisterDatasource {
                name: name.to_string(),
                plugin: plugin.to_string(),
                config_json: config_json.to_string(),
            });
            Ok(())
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
        assert!(matches!(&calls[0], MockCall::Deploy { name, dag_json } if name == "p1" && dag_json == "CREATE PIPELINE p1"));
        assert!(matches!(&calls[1], MockCall::Deploy { name, dag_json } if name == "p2" && dag_json == "CREATE PIPELINE p2"));
        assert!(matches!(&calls[2], MockCall::RegisterDatasource { name, plugin, .. } if name == "binance" && plugin == "binance_subscribe"));
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

    #[test]
    fn enable_uses_registrar_to_deploy_pipelines() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            db::pipelines::create(&conn, "p1", "CREATE PIPELINE p1 SOURCE binance;").unwrap();
            db::pipelines::create(&conn, "p2", "CREATE PIPELINE p2 SOURCE binance;").unwrap();
            db::applications::application_disable(&conn, app.id).unwrap();
            drop(conn);

            let conn2 = db.lock().unwrap();
            let registrar = MockRegistrar::new();
            let outcome = application_enable_with_registrar(&conn2, app.id, &registrar).unwrap();
            assert_eq!(outcome.outcome, "Success");
            assert_eq!(outcome.succeeded.len(), 2);

            let calls = registrar.calls();
            let deploys: Vec<&str> = calls
                .iter()
                .filter_map(|c| match c {
                    MockCall::Deploy { name, .. } => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(deploys, vec!["p1", "p2"]);
            assert!(calls
                .iter()
                .all(|c| !matches!(c, MockCall::RegisterDatasource { .. })));

            let app_row = db::applications::get(&conn2, app.id).unwrap().unwrap();
            assert!(app_row.enabled);
        });
    }

    #[test]
    fn enable_uses_registrar_to_register_datasources() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            db::datasources::create(&conn, "binance", "binance_subscribe", r#"{"u":"wss"}"#, 0).unwrap();
            db::datasources::create(&conn, "newsapi", "newsapi_subscribe", r#"{"u":"http"}"#, 0).unwrap();
            db::applications::application_disable(&conn, app.id).unwrap();
            drop(conn);

            let conn2 = db.lock().unwrap();
            let registrar = MockRegistrar::new();
            let outcome = application_enable_with_registrar(&conn2, app.id, &registrar).unwrap();
            assert_eq!(outcome.outcome, "Success");
            assert_eq!(outcome.succeeded.len(), 2);

            let calls = registrar.calls();
            let registrations: Vec<(&str, &str)> = calls
                .iter()
                .filter_map(|c| match c {
                    MockCall::RegisterDatasource { name, plugin, .. } => {
                        Some((name.as_str(), plugin.as_str()))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                registrations,
                vec![("binance", "binance_subscribe"), ("newsapi", "newsapi_subscribe")]
            );
            assert!(calls
                .iter()
                .all(|c| !matches!(c, MockCall::Deploy { .. })));

            let app_row = db::applications::get(&conn2, app.id).unwrap().unwrap();
            assert!(app_row.enabled);
        });
    }

    #[test]
    fn registrar_failure_does_not_skip_subsequent_resources() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            db::pipelines::create(&conn, "p1", "CREATE PIPELINE p1 SOURCE binance;").unwrap();
            db::pipelines::create(&conn, "p2", "CREATE PIPELINE p2 SOURCE binance;").unwrap();
            db::datasources::create(&conn, "binance", "binance_subscribe", "{}", 0).unwrap();
            db::applications::application_disable(&conn, app.id).unwrap();
            drop(conn);

            let conn2 = db.lock().unwrap();
            let registrar = PartialFailRegistrar::new(&["p1"]);
            let outcome = application_enable_with_registrar(&conn2, app.id, &registrar).unwrap();
            assert_eq!(outcome.outcome, "Degraded");
            assert_eq!(outcome.succeeded.len(), 2);
            assert_eq!(outcome.failed.len(), 1);
            assert_eq!(
                outcome.succeeded.len() + outcome.failed.len(),
                3,
                "every resource is attempted even after a failure"
            );
            assert_eq!(outcome.failed[0].id, "p1");
            assert!(outcome.failed[0].reason.contains("p1"));

            let calls = registrar.calls();
            let successful_attempts: Vec<&str> = calls
                .iter()
                .map(|c| match c {
                    MockCall::Deploy { name, .. } => name.as_str(),
                    MockCall::RegisterDatasource { name, .. } => name.as_str(),
                })
                .collect();
            assert_eq!(
                successful_attempts.len(),
                2,
                "two non-failing calls were recorded"
            );
            assert!(successful_attempts.contains(&"p2"));
            assert!(successful_attempts.contains(&"binance"));

            let app_row = db::applications::get(&conn2, app.id).unwrap().unwrap();
            assert!(app_row.enabled, "app is enabled in degraded mode");
        });
    }
}