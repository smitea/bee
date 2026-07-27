//! S33.3: end-to-end test for the AdminServer
//! write-path arms added in Task 2. Spins up an
//! AdminServer in the same process, sends each
//! of the 3 new admin RPCs (KvPut, Deploy,
//! RegisterDatasource), asserts the replies.
//!
//! The AdminServer is given a fresh in-memory
//! KVStateMachine + ControlPlaneStateMachine +
//! NodeState (no Node is actually run; the
//! placeholder state is fine for the round-trip
//! test).
//!
//! Run with: cargo test -p bee-control --test admin_write_roundtrip -- --nocapture

use std::sync::Arc;

use bee_control::control_plane::ControlPlaneStateMachine;
use bee_control::kv::KVStateMachine;
use bee_control::raft::admin_client::AdminClient;
use bee_control::raft::admin_protocol::{AdminRequest, AdminResponse};
use bee_control::raft::admin_server::AdminServer;
use bee_control::raft::node::NodeState;
use tokio::sync::Mutex;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_kv_put_roundtrip() {
    let kv = Arc::new(Mutex::new(KVStateMachine::new()));
    let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new(std::sync::Arc::new(std::sync::Mutex::new(bee_registry::PluginManager::new())))));
    let state = Arc::new(Mutex::new(NodeState::default()));
    let mut admin = AdminServer::start(
        "127.0.0.1:0".parse().unwrap(),
        kv.clone(),
        cp.clone(),
        state,
        None,
        None,
        None,
        None,  // plugin_manager (S33.5.2)
    )
    .await
    .expect("AdminServer::start");
    let addr = admin.local_addr();
    let mut client = AdminClient::connect(addr).await.expect("connect");
    let resp = client
        .call(AdminRequest::KvPut {
            key: "soak/run_1/tick_1".to_string(),
            value: b"hello".to_vec(),
        })
        .await
        .expect("KvPut call");
    assert!(matches!(resp, AdminResponse::KvPutAck { ok: true }));
    // Read back via list
    let resp = client
        .call(AdminRequest::ListKv {
            prefix: "soak/".to_string(),
        })
        .await
        .expect("ListKv call");
    if let AdminResponse::KvList(entries) = resp {
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "soak/run_1/tick_1");
        assert_eq!(entries[0].1, b"hello");
    } else {
        panic!("expected KvList");
    }
    admin.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_register_datasource_no_plugin_manager() {
    // S33.5.2: with plugin_manager = None,
    // the validation chain returns the
    // "plugin_manager not wired" error
    // before the happy-path code runs. The
    // happy path is now in
    // admin_datasource_validation.rs
    // (test: register_datasource_full_happy_path).
    let kv = Arc::new(Mutex::new(KVStateMachine::new()));
    let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new(std::sync::Arc::new(std::sync::Mutex::new(bee_registry::PluginManager::new())))));
    let state = Arc::new(Mutex::new(NodeState::default()));
    let mut admin = AdminServer::start(
        "127.0.0.1:0".parse().unwrap(),
        kv,
        cp,
        state,
        None,
        None,
        None,
        None,  // plugin_manager
    )
    .await
    .expect("AdminServer::start");
    let addr = admin.local_addr();
    let mut client = AdminClient::connect(addr).await.expect("connect");
    let resp = client
        .call(AdminRequest::RegisterDatasource {
            name: "binance".to_string(),
            adapter: "binance".to_string(),
            plugin_version: "1.0.0".to_string(),
            config_json: "{}".to_string(),
            tenant: 0,
            owner_node: 1,
        })
        .await
        .expect("RegisterDatasource call");
    if let AdminResponse::RegisterDatasourceAck { ok, error_msg } = resp {
        assert!(!ok, "expected ok=false with no plugin_manager");
        assert!(
            error_msg.contains("plugin_manager not wired"),
            "expected 'plugin_manager not wired' error, got: {error_msg}"
        );
    } else {
        panic!("expected RegisterDatasourceAck");
    }
    admin.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_deploy_roundtrip() {
    // S33.5.3: the Deploy arm extracts the
    // phase DAG and writes 1 Job + N
    // Tasks to the control plane. `SELECT
    // 1` is a single-SELECT SQL → 1 phase
    // → 1 Task.
    let kv = Arc::new(Mutex::new(KVStateMachine::new()));
    let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new(std::sync::Arc::new(std::sync::Mutex::new(bee_registry::PluginManager::new())))));
    let state = Arc::new(Mutex::new(NodeState::default()));
    let mut admin = AdminServer::start(
        "127.0.0.1:0".parse().unwrap(),
        kv,
        cp,
        state,
        None,
        None,
        None,
        None, // plugin_manager
    )
    .await
    .expect("AdminServer::start");
    let addr = admin.local_addr();
    let mut client = AdminClient::connect(addr)
        .await
        .expect("connect");
    let resp = client
        .call(AdminRequest::Deploy {
            sql_text: "SELECT 1".to_string(),
            owner_node: 1,
        })
        .await
        .expect("Deploy call");
    let (job_id, task_ids, error_msg) = match resp {
        AdminResponse::DeployAck { job_id, task_ids, error_msg } => {
            (job_id, task_ids, error_msg)
        }
        other => panic!("expected DeployAck, got: {other:?}"),
    };
    assert_eq!(job_id, 1, "expected job_id=1 for first deploy");
    assert_eq!(task_ids.len(), 1, "expected 1 task for 'SELECT 1'");
    assert_eq!(task_ids[0], 1);
    assert!(
        error_msg.is_empty(),
        "expected empty error_msg, got: {error_msg}"
    );
    admin.shutdown();
}
