//! Integration test: connection_smoke.
//!
//! S-1c: now wired to the public
//! `bee_control::test_utils::TestCluster` harness. Boots an
//! in-process 3-node Raft cluster with one AdminServer per
//! node, then exercises the GUI-side `AdminClient`:
//!   1. `AdminClient::connect(addr)` returns within 3s.
//!   2. `AdminClient::call(AdminRequest::Ping)` returns
//!      `AdminResponse::Pong`.

use std::time::Duration;

use bee_control::raft::admin_client::AdminClient;
use bee_control::raft::admin_protocol::{AdminRequest, AdminResponse};
use bee_control::test_utils::TestCluster;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connect_and_ping_succeeds() {
    let tc = TestCluster::boot_3_node_with_admin().await;
    let addr = tc.connect_addr();

    let result = tokio::time::timeout(
        Duration::from_secs(3),
        AdminClient::connect(addr),
    )
    .await;
    assert!(
        result.is_ok(),
        "AdminClient::connect must complete within 3s"
    );
    let mut client = result.unwrap().expect("connect handshake");
    let pong = client
        .call(AdminRequest::Ping)
        .await
        .expect("AdminClient::call(Ping)");
    assert!(
        matches!(pong, AdminResponse::Pong),
        "expected Pong, got {pong:?}"
    );
}