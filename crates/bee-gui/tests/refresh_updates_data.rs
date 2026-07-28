//! Integration test: refresh_updates_data.
//!
//! S-1c: now wired to the public
//! `bee_control::test_utils::TestCluster` harness. Verifies
//! that after `submit_kv` writes a key to the cluster, a
//! `ClusterStatus` admin RPC returns a non-empty
//! `nodes` vector (the leader's view of itself), and the
//! `ListKv` admin RPC round-trips the value back.

use std::time::Duration;

use bee_control::kv::Op;
use bee_control::raft::admin_client::AdminClient;
use bee_control::raft::admin_protocol::{
    AdminRequest, AdminResponse,
};
use bee_control::test_utils::TestCluster;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refresh_updates_recent_jobs_count() {
    let tc = TestCluster::boot_3_node_with_admin().await;
    let addr = tc.connect_addr();

    // Wait for leader election so submit_kv has a target.
    let _leader = tc
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader must be elected within 3s");

    // Submit one KV write via the cluster harness.
    tc.submit_kv(Op::Put {
        key: "soak/run_1/tick_1".to_string(),
        value: b"hello".to_vec(),
    })
    .await
    .expect("submit_kv to leader must succeed");

    // Connect to a node's admin port and ask for the
    // cluster status; the response's `nodes` vector is
    // non-empty (the local node's view of itself).
    let mut client = tokio::time::timeout(
        Duration::from_secs(3),
        AdminClient::connect(addr),
    )
    .await
    .expect("connect within 3s")
    .expect("connect handshake");

    let resp = client
        .call(AdminRequest::ClusterStatus)
        .await
        .expect("ClusterStatus call");
    let metrics = match resp {
        AdminResponse::ClusterMetrics(m) => m,
        other => panic!("expected ClusterMetrics, got {other:?}"),
    };
    assert!(
        !metrics.nodes.is_empty(),
        "ClusterStatus must report at least one node (got: {metrics:?})"
    );

    // Read back via ListKv to confirm the write landed.
    let list = client
        .call(AdminRequest::ListKv {
            prefix: "soak/".to_string(),
        })
        .await
        .expect("ListKv call");
    let entries = match list {
        AdminResponse::KvList(es) => es,
        other => panic!("expected KvList, got {other:?}"),
    };
    assert_eq!(entries.len(), 1, "expected exactly one KV entry");
    assert_eq!(entries[0].0, "soak/run_1/tick_1");
    assert_eq!(entries[0].1, b"hello".to_vec());
}