//! Integration test: connection_lost_recovery.
//!
//! S-1c: now wired to the public
//! `bee_control::test_utils::TestCluster` harness.
//!
//! Two halves:
//!
//! 1. **Lost connection** — after `tc.shutdown_node(leader)`
//!    tears down the leader's `AdminServer` listener, a fresh
//!    `AdminClient::connect(leader_addr)` fails with an
//!    `AdminError::Io` (the OS returns `ECONNREFUSED`).
//!    This is the observable the GUI's reconnect loop
//!    reacts to.
//! 2. **Recovery** — once we reconnect to one of the
//!    *surviving* nodes, `AdminClient::call(Ping)` succeeds.
//!    This is the observable the GUI's success state
//!    rebuilds from.
//!
//! Note: the GUI's reconnect-on-failure state machine is
//! exercised by the `connection::tests` unit tests in
//! `src/connection.rs`; this integration test focuses on
//! the transport-level observations.

use std::time::Duration;

use bee_control::raft::admin_client::{AdminClient, AdminError};
use bee_control::raft::admin_protocol::AdminRequest;
use bee_control::test_utils::TestCluster;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kill_node_then_retry_restores_connection() {
    let tc = TestCluster::boot_3_node_with_admin().await;
    let leader = tc
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader must be elected within 3s");
    let leader_addr = tc.admin_addrs[&leader];

    // 1. The leader is alive; a connect + Ping round-trip
    // succeeds.
    let mut client = tokio::time::timeout(
        Duration::from_secs(3),
        AdminClient::connect(leader_addr),
    )
    .await
    .expect("connect within 3s")
    .expect("connect handshake");
    client
        .call(AdminRequest::Ping)
        .await
        .expect("Ping to alive leader");

    // 2. Shutdown the leader (Node + AdminServer).
    tc.shutdown_node(leader).await;

    // 3. A fresh AdminClient::connect to the dead
    // leader's address fails with AdminError::Io.
    let res = tokio::time::timeout(
        Duration::from_secs(3),
        AdminClient::connect(leader_addr),
    )
    .await
    .expect("connect attempt within 3s");
    let err = match res {
        Ok(_) => panic!("connect to dead leader unexpectedly succeeded"),
        Err(e) => e,
    };
    assert!(
        matches!(err, AdminError::Io(_)),
        "expected AdminError::Io when connecting to a shut-down node, got: {err:?}"
    );

    // 4. Recovery: connect to a surviving node's
    // admin port. Pick the lowest-id node that is
    // not the dead leader.
    let survivor_id = (1..=3u32)
        .find(|id| *id != leader)
        .expect("at least one survivor");
    let survivor_addr = tc.admin_addrs[&survivor_id];
    let mut recovered = tokio::time::timeout(
        Duration::from_secs(3),
        AdminClient::connect(survivor_addr),
    )
    .await
    .expect("reconnect within 3s")
    .expect("reconnect handshake");
    let pong = recovered
        .call(AdminRequest::Ping)
        .await
        .expect("Ping on survivor");
    assert!(
        matches!(pong, bee_control::raft::admin_protocol::AdminResponse::Pong),
        "expected Pong on survivor, got {pong:?}"
    );
}