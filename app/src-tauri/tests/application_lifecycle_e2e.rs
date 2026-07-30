use std::time::Duration;

use bee_control::raft::admin_client::AdminClient;
use bee_control::raft::admin_protocol::{AdminRequest, AdminResponse};
use bee_control::test_utils::TestCluster;
use bee_plugin_sdk::{AdapterDescriptor, PluginManifest, PluginName};
use bee_registry::PluginManager;

use app_lib::commands::applications::{
    application_enable_with_registrar, AdminServerDeployRegistrar,
};
use app_lib::db;

fn stub_plugin_manager() -> std::sync::Arc<PluginManager> {
    let mut pm = PluginManager::new();
    let binance_manifest = PluginManifest {
        name: PluginName("binance_subscribe".into()),
        feature_version: "0.0.1".into(),
        abi_version: "v1".into(),
        adapters: vec![AdapterDescriptor {
            name: "binance_subscribe".into(),
            is_input: true,
        }],
        handlers: vec![],
    };
    pm.register(b"binance-stub-content", binance_manifest)
        .expect("register binance stub");
    let newsapi_manifest = PluginManifest {
        name: PluginName("newsapi_subscribe".into()),
        feature_version: "0.0.1".into(),
        abi_version: "v1".into(),
        adapters: vec![AdapterDescriptor {
            name: "newsapi_subscribe".into(),
            is_input: true,
        }],
        handlers: vec![],
    };
    pm.register(b"newsapi-stub-content", newsapi_manifest)
        .expect("register newsapi stub");
    std::sync::Arc::new(pm)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enable_lifecycle_calls_register_datasource_and_deploy_against_live_cluster() {
    let pm = stub_plugin_manager();
    let tc = TestCluster::boot_3_node_with_admin_and_plugins(Some(pm)).await;
    tc.wait_for_leader(Duration::from_secs(5))
        .await
        .expect("cluster must elect a leader within 5s");
    let admin_addr = tc
        .admin_addrs
        .get(&1)
        .copied()
        .expect("node 1 admin addr");

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("bee-client.sqlite");
    let database = std::sync::Arc::new(db::Database::open(&db_path).expect("Database::open"));

    let db_for_setup = std::sync::Arc::clone(&database);
    let app_id = tokio::task::spawn_blocking(move || {
        let conn = db_for_setup.lock().expect("db lock");
        db::settings::put(&conn, "addr", &admin_addr.to_string())
            .expect("set admin addr in client_settings");
        let app = db::applications::create(&conn, "alpha").expect("create app");
        db::datasources::create(&conn, "binance", "binance_subscribe", r#"{"u":"wss://x"}"#, 0)
            .expect("ds binance");
        db::datasources::create(
            &conn,
            "newsapi",
            "newsapi_subscribe",
            r#"{"u":"http://x"}"#,
            0,
        )
        .expect("ds newsapi");
        db::pipelines::create(&conn, "p1", "SELECT 1").expect("pipeline p1");
        db::pipelines::create(&conn, "p2", "SELECT 2").expect("pipeline p2");
        db::applications::application_disable(&conn, app.id).expect("disable");
        app.id
    })
    .await
    .expect("join setup");

    let db_for_enable = std::sync::Arc::clone(&database);
    let outcome = tokio::task::spawn_blocking(move || {
        let conn = db_for_enable.lock().expect("db lock");
        let registrar = AdminServerDeployRegistrar {
            addr: admin_addr,
            tenant: 0,
        };
        application_enable_with_registrar(&conn, app_id, &registrar).expect("enable")
    })
    .await
    .expect("join enable");

    assert_eq!(
        outcome.outcome,
        "Success",
        "expected Success, got {:?}; succeeded={:?}, failed={:?}",
        outcome.outcome, outcome.succeeded, outcome.failed
    );
    assert_eq!(outcome.succeeded.len(), 4);
    assert!(outcome.failed.is_empty());

    let db_for_read = std::sync::Arc::clone(&database);
    let app_row = tokio::task::spawn_blocking(move || {
        let conn = db_for_read.lock().expect("db lock");
        db::applications::get(&conn, app_id).expect("get").expect("row")
    })
    .await
    .expect("join read");

    assert!(app_row.enabled);

    let mut client = AdminClient::connect(admin_addr)
        .await
        .expect("connect to node 1");
    let resp = client.call(AdminRequest::ListJobs).await.expect("ListJobs");
    let jobs = match resp {
        AdminResponse::JobList(js) => js,
        other => panic!("expected JobList, got {other:?}"),
    };
    assert_eq!(
        jobs.len(),
        2,
        "two pipelines must appear in ListJobs, got {jobs:?}"
    );

    let resp = client
        .call(AdminRequest::ListKv {
            prefix: "ds/".to_string(),
        })
        .await
        .expect("ListKv");
    let entries = match resp {
        AdminResponse::KvList(es) => es,
        other => panic!("expected KvList, got {other:?}"),
    };
    let keys: Vec<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    assert!(
        keys.iter().any(|k| k.contains("binance")),
        "binance must be in KV ds/ prefix, got {keys:?}"
    );
    assert!(
        keys.iter().any(|k| k.contains("newsapi")),
        "newsapi must be in KV ds/ prefix, got {keys:?}"
    );

    let db_for_audit = std::sync::Arc::clone(&database);
    let events = tokio::task::spawn_blocking(move || {
        let conn = db_for_audit.lock().expect("db lock");
        db::audit::query(&conn, Some(app_id), 100).expect("audit query")
    })
    .await
    .expect("join audit");

    let actions: Vec<&str> = events.iter().map(|e| e.action.as_str()).collect();
    assert!(
        actions.iter().any(|a| *a == "application.enable"),
        "enable audit events must be recorded, got {actions:?}"
    );
    let per_resource_enable: Vec<_> = events
        .iter()
        .filter(|e| e.action == "application.enable" && e.resource_id.is_some())
        .collect();
    assert_eq!(
        per_resource_enable.len(),
        4,
        "one per-resource enable audit event per resource, got {events:?}"
    );

    drop(tc);
}