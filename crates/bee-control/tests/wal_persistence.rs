use std::path::PathBuf;
use std::time::Duration;

use bee_control::kv::Op;
use bee_control::test_utils::TestCluster;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wal_persists_log_and_term_across_process_restart() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let log_path: PathBuf = tempdir.path().to_path_buf();

    let leader_first_run = {
        let tc = TestCluster::boot_3_node_with_wal(&log_path).await;
        let cluster = tc.cluster.clone();
        let _keep_alive = tc;

        let leader = cluster
            .wait_for_leader(Duration::from_secs(5))
            .await
            .expect("leader elected");

        let handle = cluster.node(leader).expect("leader handle");
        let (tx, rx) = tokio::sync::oneshot::channel();
        handle
            .node_transport
            .submit_command(bee_control::raft::types::NodeCommand::Submit {
                op: Op::Put {
                    key: "alpha".to_string(),
                    value: b"first".to_vec(),
                },
                reply: tx,
            })
            .await
            .expect("submit_command to leader");
        let apply_result = rx.await.expect("leader reply");
        apply_result.expect("leader apply result");

        cluster
            .wait_for_log_converge("alpha", b"first", Duration::from_secs(5))
            .await;

        let term_before = {
            let state = handle.state.lock().await;
            (state.current_term, state.log.len())
        };
        assert!(term_before.1 >= 1, "leader should have at least one entry");

        leader
    };

    // S-1c: rebuild the cluster from the same WAL dir.
    // TestCluster::boot_3_node_with_wal wraps the same
    // Cluster::new(ClusterConfig { log_path }) call.
    let tc = TestCluster::boot_3_node_with_wal(&log_path).await;
    let restored = tc.cluster.clone();
    let _keep_alive = tc;

    let leader_handle = restored.node(leader_first_run).expect("leader handle");
    let restored_state = leader_handle.state.lock().await;
    let restored_kv = leader_handle.kv.lock().await;
    assert!(
        !restored_state.log.is_empty(),
        "expected replayed log to contain the put entry"
    );
    assert_eq!(
        restored_kv.get("alpha").as_deref(),
        Some(b"first".as_slice()),
        "kv state machine should retain the put"
    );
    assert!(
        restored_state.current_term >= 1,
        "expected replayed term >= 1, got {}",
        restored_state.current_term
    );
}
