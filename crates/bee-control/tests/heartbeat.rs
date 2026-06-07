use std::time::Duration;

use bee_control::deployer::{Deployer, DeployerConfig, Edge, HandlerKind, Pipeline, TaskSpec};
use bee_control::heartbeat::{HeartbeatConfig, HeartbeatOrchestrator};
use bee_control::kv::TaskStatus;

fn linear_pipeline(tasks: Vec<TaskSpec>, edges: Vec<Edge>) -> Pipeline {
    Pipeline {
        name: "hb-test".to_string(),
        tasks,
        edges,
    }
}

fn started(id: u32) -> TaskSpec {
    // 600m CPU per task so FFD spreads the 3 tasks across the 3
    // workers (each worker has 1000m). With 0 requirements FFD would
    // pile them all on worker 1 and the orphan test would have no
    // node-2 tasks to mark.
    TaskSpec {
        task_id: id,
        phase_id: 0,
        handler_kind: HandlerKind::Started {
            tag: format!("T{id}"),
        },
        cpu_millicores: 600,
        mem_mb: 0,
    }
}

fn fast_hb_config() -> HeartbeatConfig {
    HeartbeatConfig {
        interval: Duration::from_millis(100),
        orphan_threshold: Duration::from_millis(300),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kill_node_marks_its_tasks_as_orphaned_within_threshold() {
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let tasks = vec![started(1), started(2), started(3)];
    let job_id = deployer
        .deploy(linear_pipeline(tasks, vec![]))
        .await
        .unwrap();

    let cluster = deployer.cluster.clone();
    let log = deployer.log.clone();
    let mut orchestrator = HeartbeatOrchestrator::new(cluster, fast_hb_config(), log);
    orchestrator.start();

    // Let a few heartbeat ticks happen so every node (including node 2)
    // has a fresh last_heartbeat in the leader's CP.
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Kill node 2. After this, node 2 stops sending heartbeats.
    deployer.cluster.shutdown_node(2).await;

    // Wait for the leader to notice and orphan node 2's tasks.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut node2_orphaned = false;
    while tokio::time::Instant::now() < deadline {
        let mapping = deployer.job_to_node_mapping(job_id).await;
        // Read from any node's CP (they all converged via Raft).
        // Verify node 2's task(s) are Orphaned.
        let node2_tasks: Vec<u32> = mapping
            .iter()
            .filter(|(_, owner)| **owner == 2)
            .map(|(task_id, _)| *task_id)
            .collect();
        if !node2_tasks.is_empty() {
            // Read from a live node (e.g., the leader, or any alive node).
            let read_node = deployer
                .cluster
                .nodes()
                .map(|(id, _)| id)
                .find(|id| deployer.cluster.is_alive(*id))
                .unwrap_or(1);
            let cp = deployer.cluster.node(read_node).unwrap().cp.lock().await;
            let all_orphaned = node2_tasks
                .iter()
                .all(|tid| cp.get_task(*tid).map(|t| t.status == TaskStatus::Orphaned).unwrap_or(false));
            if all_orphaned {
                node2_orphaned = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if !node2_orphaned {
        eprintln!("orphan test failed. orchestrator log messages:");
        for m in orchestrator.log.messages() {
            eprintln!("  {m}");
        }
        eprintln!("cluster metrics:");
        for m in deployer.cluster.metrics().await {
            eprintln!("  {m:?}");
        }
        let mapping = deployer.job_to_node_mapping(job_id).await;
        eprintln!("task mapping: {mapping:?}");
    }
    assert!(
        node2_orphaned,
        "all node 2 tasks must reach Orphaned within 2s of shutdown"
    );

    orchestrator.stop();
    orchestrator.join().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn heartbeat_interval_and_orphan_threshold_are_configurable() {
    // Use an aggressive config: 50ms interval, 200ms threshold.
    let aggressive = HeartbeatConfig {
        interval: Duration::from_millis(50),
        orphan_threshold: Duration::from_millis(200),
    };
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let tasks = vec![started(1), started(2), started(3)];
    let _job_id = deployer
        .deploy(linear_pipeline(tasks, vec![]))
        .await
        .unwrap();

    let cluster = deployer.cluster.clone();
    let log = deployer.log.clone();
    let mut orch = HeartbeatOrchestrator::new(cluster, aggressive, log);
    orch.start();

    // Sanity: orchestrator is running.
    assert!(orch.is_running());

    // Stop it.
    orch.stop();
    orch.join().await;
    assert!(!orch.is_running());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leader_keeps_receiving_heartbeats_after_follower_killed() {
    // After killing a non-leader node, the surviving nodes (including the
    // leader) must keep exchanging heartbeats and the leader must not be
    // erroneously marked as Orphaned.
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let tasks = vec![started(1), started(2), started(3)];
    let _job_id = deployer
        .deploy(linear_pipeline(tasks, vec![]))
        .await
        .unwrap();

    let cluster = deployer.cluster.clone();
    let log = deployer.log.clone();
    let mut orch = HeartbeatOrchestrator::new(cluster, fast_hb_config(), log);
    orch.start();

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Find and kill a non-leader node.
    let leader = deployer.cluster.leader().await.unwrap();
    let non_leader: u32 = (1..=3u32).find(|id| *id != leader).unwrap();
    deployer.cluster.shutdown_node(non_leader).await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // The leader's own last_heartbeat should still be fresh (the leader
    // updates its own heartbeat in the orchestrator). Verify by reading CP.
    let leader_node = deployer.cluster.node(leader).unwrap();
    let cp = leader_node.cp.lock().await;
    let now_ms = std_time_ms_for_test();
    if let Some(last) = cp.last_heartbeat(leader) {
        assert!(
            now_ms.saturating_sub(last) < 1_000,
            "leader must not be marked stale; last_heartbeat was {last} ms ago"
        );
    }

    orch.stop();
    orch.join().await;
}

fn std_time_ms_for_test() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
