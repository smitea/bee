use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bee_control::kv::Op;
use bee_control::raft::cluster::{Cluster, ClusterConfig};
use bee_control::raft::snapshot::SnapshotStore;
use bee_control::test_utils::TestCluster;

/// S07-x: a 3-node cluster that writes enough committed
/// entries to trigger at least one snapshot, then
/// restarts from snapshot + WAL tail. The KV state
/// machine must survive the restart; the WAL must be
/// measurably smaller than it would have been without
/// snapshotting; the snapshot file must be readable via
/// `SnapshotStore::latest` and carry a non-zero
/// `last_included_index`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_then_restart_preserves_kv_state() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let log_path: PathBuf = tempdir.path().join("wal");
    let snap_path: PathBuf = tempdir.path().join("snap");
    std::fs::create_dir_all(&log_path).expect("create wal dir");
    std::fs::create_dir_all(&snap_path).expect("create snap dir");

    // First run: submit 10 Puts with aggressive
    // snapshot triggers so we definitely get at
    // least 2 snapshots (one every 5 entries +
    // one every 100ms).
    let leader_first_run = {
        let config = ClusterConfig {
            n: 3,
            base_election_timeout: Duration::from_millis(800),
            heartbeat_interval: Duration::from_millis(100),
            nodes: Vec::new(),
            plugin_manager: None,
            log_path: Some(log_path.clone()),
            snapshot_dir: Some(snap_path.clone()),
            snapshot_threshold: 5,
            snapshot_interval: Duration::from_millis(100),
        };
        let cluster = Cluster::new(config).await;
        let leader = cluster
            .wait_for_leader(Duration::from_secs(5))
            .await
            .expect("leader elected");

        for i in 0..10 {
            let kv_key = format!("k{i}");
            let kv_val = format!("v{i}").into_bytes();
            cluster
                .submit(
                    leader,
                    Op::Put {
                        key: kv_key,
                        value: kv_val,
                    },
                )
                .await
                .expect("submit put");
        }

        cluster
            .wait_for_log_converge("k9", b"v9", Duration::from_secs(5))
            .await;

        // Drop the cluster cleanly.
        for id in 1..=3u32 {
            cluster.shutdown_node(id).await;
        }
        leader
    };

    // Inspect on-disk state. The WAL should have
    // been truncated by at least one snapshot; the
    // snapshot file should exist.
    let snap_files: Vec<u64> = {
        let store = SnapshotStore::open(&snap_path).expect("SnapshotStore::open");
        store.list().expect("list")
    };
    assert!(
        !snap_files.is_empty(),
        "expected at least one snapshot file in {}, got {:?}",
        snap_path.display(),
        snap_files
    );
    let latest_snap_index = *snap_files.iter().max().expect("max");
    assert!(
        latest_snap_index >= 5,
        "expected last_included_index >= 5, got {latest_snap_index}"
    );

    let latest = {
        let store = SnapshotStore::open(&snap_path).expect("SnapshotStore::open");
        store.latest().expect("latest").expect("latest present")
    };
    assert!(
        latest.last_included_index >= 5,
        "snapshot last_included_index too small: {}",
        latest.last_included_index
    );
    assert!(
        latest.log.len() as u64 >= latest.last_included_index,
        "snapshot log should contain entries up to last_included_index: log.len()={}, last_included_index={}",
        latest.log.len(),
        latest.last_included_index
    );

    let wal_size_before = std::fs::metadata(log_path.join("node-1.wal"))
        .expect("wal exists")
        .len();
    assert!(
        wal_size_before < 4096,
        "WAL should have been compacted by snapshotting (got {wal_size_before} bytes; \
         without snapshots it would carry all 10 Put entries)"
    );

    // Second run: rebuild the cluster from the
    // same WAL + snapshot dirs.
    {
        let config = ClusterConfig {
            n: 3,
            base_election_timeout: Duration::from_millis(800),
            heartbeat_interval: Duration::from_millis(100),
            nodes: Vec::new(),
            plugin_manager: None,
            log_path: Some(log_path.clone()),
            snapshot_dir: Some(snap_path.clone()),
            snapshot_threshold: 5,
            snapshot_interval: Duration::from_millis(100),
        };
        let cluster = Cluster::new(config).await;

        // Wait for a fresh leader, then verify
        // every key written in run #1 is
        // present in the leader's KV.
        let leader = cluster
            .wait_for_leader(Duration::from_secs(5))
            .await
            .expect("leader elected after restart");
        let handle = cluster.node(leader).expect("leader handle");
        let kv = handle.kv.lock().await;
        for i in 0..10 {
            let key = format!("k{i}");
            let expected = format!("v{i}").into_bytes();
            let actual = kv.get(&key);
            assert_eq!(
                actual.as_deref(),
                Some(expected.as_slice()),
                "key {key} missing or wrong after snapshot+wal restart"
            );
        }
        // The leader's in-memory log should also
        // have all 10 entries (they came from
        // snapshot.log + WAL tail).
        let state = handle.state.lock().await;
        assert_eq!(
            state.log.len(),
            10,
            "expected 10 log entries after snapshot+wal replay, got {}",
            state.log.len()
        );
        assert_eq!(
            state.last_snapshot_index, latest_snap_index,
            "expected last_snapshot_index={latest_snap_index}, got {}",
            state.last_snapshot_index
        );
    }

    // Reference the leader id so the variable
    // isn't flagged as unused if the test body
    // is ever reorganised.
    let _ = leader_first_run;
}

/// S07-x: SnapshotStore round-trips a snapshot
/// through `write` + `read`. Locks down the
/// file format contract: magic header,
/// length-prefixed bincode.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_store_write_read_roundtrip() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let snap_path = tempdir.path().join("snap");
    let store = Arc::new(std::sync::Mutex::new(
        SnapshotStore::open(&snap_path).expect("open"),
    ));

    let entries: Vec<bee_control::raft::types::LogEntry> = (0..5)
        .map(|i| {
            bee_control::raft::types::LogEntry::new(
                1,
                Op::Put {
                    key: format!("k{i}"),
                    value: format!("v{i}").into_bytes(),
                },
            )
        })
        .collect();
    let snap = bee_control::raft::snapshot::Snapshot {
        last_included_index: 5,
        last_included_term: 1,
        current_term: 1,
        voted_for: Some(2),
        log: entries,
    };
    store.lock().expect("poisoned").write(&snap).expect("write");

    let indices = store.lock().expect("poisoned").list().expect("list");
    assert_eq!(indices, vec![5]);

    let read_back = store
        .lock()
        .expect("poisoned")
        .read(5)
        .expect("read");
    assert_eq!(read_back.last_included_index, 5);
    assert_eq!(read_back.last_included_term, 1);
    assert_eq!(read_back.current_term, 1);
    assert_eq!(read_back.voted_for, Some(2));
    assert_eq!(read_back.log.len(), 5);

    // `latest` should return the same snapshot.
    let latest = store
        .lock()
        .expect("poisoned")
        .latest()
        .expect("latest")
        .expect("present");
    assert_eq!(latest.last_included_index, 5);
    assert_eq!(latest.log.len(), 5);
}

/// S07-x: Verify that the `TestCluster::boot_3_node_with_wal`
/// helper still works (it predates snapshot support and
/// must NOT need snapshot fields) — a regression
/// guard for the default ClusterConfig shape.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_cluster_wal_helper_unchanged() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let tc = TestCluster::boot_3_node_with_wal(tempdir.path()).await;
    let _leader = tc
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader elected");
}