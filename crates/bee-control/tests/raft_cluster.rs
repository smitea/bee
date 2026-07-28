use std::time::Duration;

use bee_control::raft::Role;
use bee_control::test_utils::TestCluster;
use bee_control::Op;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_node_cluster_elects_exactly_one_leader() {
    let tc = TestCluster::boot_3_node_with_admin().await;
    let cluster = tc.cluster.clone();
    let _keep_alive = tc; // shut down admin_servers on drop
    let leader = cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("cluster must elect a leader within 3s");

    let metrics = cluster.metrics().await;
    let leaders: Vec<_> = metrics
        .iter()
        .filter(|m| m.role == Role::Leader)
        .map(|m| m.id)
        .collect();
    assert_eq!(leaders, vec![leader], "exactly one leader expected");

    let followers: Vec<_> = metrics
        .iter()
        .filter(|m| m.role == Role::Follower)
        .map(|m| m.id)
        .collect();
    assert_eq!(followers.len(), 2, "two followers expected");

    for m in &metrics {
        assert!(m.term >= 1, "term must advance from 0 after election");
        assert_eq!(m.leader_id, Some(leader));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kill_leader_triggers_reelection_within_2s() {
    let tc = TestCluster::boot_3_node_with_admin().await;
    let cluster = tc.cluster.clone();
    let _keep_alive = tc;
    let old_leader = cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("initial leader must be elected");

    cluster.shutdown_node(old_leader).await;

    let new_leader = cluster
        .wait_for_leader(Duration::from_secs(2))
        .await
        .expect("new leader must be elected within 2s after killing old one");
    assert_ne!(new_leader, old_leader, "new leader must differ from killed one");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn submitted_put_replicates_to_all_nodes() {
    let tc = TestCluster::boot_3_node_with_admin().await;
    let cluster = tc.cluster.clone();
    let _keep_alive = tc;
    let leader = cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader must be elected");

    cluster
        .submit(
            leader,
            Op::Put {
                key: "alpha".to_string(),
                value: b"hello".to_vec(),
            },
        )
        .await
        .expect("submit to leader must succeed");

    let converged = cluster
        .wait_for_log_converge("alpha", b"hello", Duration::from_secs(3))
        .await;
    assert!(converged, "all nodes must converge to the submitted value");

    for id in 1..=3u32 {
        let kv = cluster.node(id).expect("node exists").kv.lock().await;
        assert_eq!(kv.get("alpha"), Some(b"hello".to_vec()));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn submit_to_non_leader_returns_error() {
    let tc = TestCluster::boot_3_node_with_admin().await;
    let cluster = tc.cluster.clone();
    let _keep_alive = tc;
    let leader = cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader must be elected");
    let follower = (1..=3u32).find(|id| *id != leader).expect("at least one follower");

    let res = cluster
        .submit(
            follower,
            Op::Put {
                key: "k".to_string(),
                value: b"v".to_vec(),
            },
        )
        .await;
    assert!(res.is_err(), "submit to follower must fail");
}
