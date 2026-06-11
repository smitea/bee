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
    let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new()));
    let state = Arc::new(Mutex::new(NodeState::default()));
    let mut admin = AdminServer::start(
        "127.0.0.1:0".parse().unwrap(),
        kv.clone(),
        cp.clone(),
        state,
        None,
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
async fn admin_register_datasource_roundtrip() {
    let kv = Arc::new(Mutex::new(KVStateMachine::new()));
    let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new()));
    let state = Arc::new(Mutex::new(NodeState::default()));
    let mut admin = AdminServer::start(
        "127.0.0.1:0".parse().unwrap(),
        kv.clone(),
        cp.clone(),
        state,
        None,
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
        assert!(ok, "expected ok=true, got error: {error_msg}");
    } else {
        panic!("expected RegisterDatasourceAck");
    }
    // Read back via list: should be in KV
    let resp = client
        .call(AdminRequest::ListKv {
            prefix: "soak/datasource/".to_string(),
        })
        .await
        .expect("ListKv call");
    if let AdminResponse::KvList(entries) = resp {
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "soak/datasource/binance");
    } else {
        panic!("expected KvList");
    }
    admin.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_deploy_roundtrip() {
    // The S33.3 MVP deploy is a marker (the
    // full bee-dsl-sql runner is S33.4). The
    // round-trip should return a DeployAck
    // with job_id=0 and a non-empty error_msg
    // (the marker note).
    let kv = Arc::new(Mutex::new(KVStateMachine::new()));
    let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new()));
    let state = Arc::new(Mutex::new(NodeState::default()));
    let mut admin = AdminServer::start(
        "127.0.0.1:0".parse().unwrap(),
        kv.clone(),
        cp.clone(),
        state,
        None,
    )
    .await
    .expect("AdminServer::start");
    let addr = admin.local_addr();
    let mut client = AdminClient::connect(addr).await.expect("connect");
    let resp = client
        .call(AdminRequest::Deploy {
            sql_text: "SELECT 1".to_string(),
            owner_node: 1,
        })
        .await
        .expect("Deploy call");
    if let AdminResponse::DeployAck { job_id, error_msg, .. } = resp {
        assert_eq!(job_id, 0);
        assert!(!error_msg.is_empty(), "expected non-empty error_msg (marker note)");
    } else {
        panic!("expected DeployAck");
    }
    admin.shutdown();
}
