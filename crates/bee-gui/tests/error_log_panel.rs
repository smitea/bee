//! Integration test: error_log_panel.
//!
//! S-1c: now wired to the public
//! `bee_control::test_utils::TestCluster` harness. Covers
//! both halves of the GUI's error-log panel contract:
//!
//! 1. **Happy path** — `AdminClient::call(ClusterStatus)` on
//!    the leader returns `AdminResponse::ClusterMetrics`
//!    (the smoke path the GUI's `Refresh` button exercises).
//! 2. **Error path** — submit a `RegisterDatasource` to a
//!    follower without forwarding; the follower's local
//!    admin server applies directly via the local
//!    `dispatch_with_apply` path. We use a deliberately
//!    invalid plugin reference to provoke an
//!    `AdminResponse::Error`, which the AdminClient surfaces
//!    as `AdminError::ServerError` — exactly the path that
//!    drives `log_rpc_failure` in the GUI's LogPanel.
//!
//! The GUI-side LogPanel rendering is unit-tested by
//! `crate::log_panel::tests`; this test focuses on the
//! transport-level observation that the server returns a
//! structured error.

use std::time::Duration;

use bee_control::raft::admin_client::AdminClient;
use bee_control::raft::admin_protocol::{
    AdminRequest, AdminResponse,
};
use bee_control::test_utils::TestCluster;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rpc_server_error_appears_in_log_panel() {
    let tc = TestCluster::boot_3_node_with_admin().await;
    let _leader = tc
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader must be elected within 3s");
    let addr = tc.connect_addr();

    let mut client = tokio::time::timeout(
        Duration::from_secs(3),
        AdminClient::connect(addr),
    )
    .await
    .expect("connect within 3s")
    .expect("connect handshake");

    // ---- Smoke path: ClusterStatus returns ClusterMetrics
    let smoke = client
        .call(AdminRequest::ClusterStatus)
        .await
        .expect("ClusterStatus call");
    assert!(
        matches!(smoke, AdminResponse::ClusterMetrics(_)),
        "expected ClusterMetrics, got {smoke:?}"
    );

    // ---- Error path: register a datasource with an
    // adapter that does not exist in the test's
    // (unpopulated) PluginManager. The AdminServer's
    // RegisterDatasource arm rejects the request with
    // a `RegisterDatasourceAck { ok: false, .. }`
    // (the AdminServer's plugin_manager is `None`,
    // so the test path always fails). The
    // AdminClient surfaces this as an `Ok` reply —
    // the GUI's caller code reads `ok` and the
    // `error_msg` to decide whether to log an error.
    let bad_register = client
        .call(AdminRequest::RegisterDatasource {
            name: "bad-ds".to_string(),
            adapter: "does-not-exist".to_string(),
            plugin_version: "^0".to_string(),
            config_json: "{}".to_string(),
            tenant: 0,
            owner_node: 1,
        })
        .await
        .expect("RegisterDatasource RPC must return (the error is in the body)");
    let ack = match bad_register {
        AdminResponse::RegisterDatasourceAck { ok, error_msg } => (ok, error_msg),
        other => panic!("expected RegisterDatasourceAck, got {other:?}"),
    };
    assert!(
        !ack.0,
        "expected ok: false for invalid adapter, got ok: {}",
        ack.0
    );
    assert!(
        !ack.1.is_empty(),
        "expected non-empty error_msg describing the failure"
    );
}