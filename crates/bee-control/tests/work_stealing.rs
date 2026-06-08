use std::time::Duration;

use bee_control::deployer::{Deployer, DeployerConfig, Edge, HandlerKind, Pipeline, TaskSpec};
use bee_control::heartbeat::{HeartbeatConfig, HeartbeatOrchestrator};
use bee_control::kv::{Op, TaskStatus};

fn linear_pipeline(tasks: Vec<TaskSpec>, edges: Vec<Edge>) -> Pipeline {
    Pipeline {
        name: "ws-test".to_string(),
        tasks,
        edges,
        stream_identities: vec![],
    }
}

fn started(id: u32) -> TaskSpec {
    // 600m CPU forces FFD to spread across the 3 workers.
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

async fn read_task_status_anywhere(
    cluster: &bee_control::Cluster,
    task_id: u32,
) -> Option<TaskStatus> {
    for (_, handle) in cluster.nodes() {
        if !cluster.is_alive(handle.id) {
            continue;
        }
        let cp = handle.cp.lock().await;
        if let Some(t) = cp.get_task(task_id) {
            return Some(t.status);
        }
    }
    None
}

async fn read_task_owner_anywhere(
    cluster: &bee_control::Cluster,
    task_id: u32,
) -> Option<u32> {
    for (_, handle) in cluster.nodes() {
        if !cluster.is_alive(handle.id) {
            continue;
        }
        let cp = handle.cp.lock().await;
        if let Some(t) = cp.get_task(task_id) {
            return Some(t.owner_node);
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn steal_task_transitions_orphaned_to_migrating_with_new_owner() {
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let tasks = vec![started(1), started(2), started(3)];
    let job_id = deployer
        .deploy(linear_pipeline(tasks, vec![]))
        .await
        .unwrap();

    // Verify which node owns task 2.
    let mapping = deployer.job_to_node_mapping(job_id).await;
    let original_owner_of_2 = mapping[&2];
    assert!(
        (1..=3u32).contains(&original_owner_of_2),
        "task 2 must be on a valid worker"
    );

    // Start heartbeat orchestrator so kill -> Orphaned.
    let cluster = deployer.cluster.clone();
    let log = deployer.log.clone();
    let mut hb = HeartbeatOrchestrator::new(
        cluster,
        HeartbeatConfig {
            interval: Duration::from_millis(100),
            orphan_threshold: Duration::from_millis(300),
        },
        log,
    );
    hb.start();

    tokio::time::sleep(Duration::from_millis(400)).await;

    // Kill the node that owns task 2.
    deployer.cluster.shutdown_node(original_owner_of_2).await;

    // Wait for task 2 to become Orphaned.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(TaskStatus::Orphaned) = read_task_status_anywhere(&deployer.cluster, 2).await {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("task 2 did not become Orphaned within 2s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // A free node (any alive node that isn't the original owner) submits
    // StealTask for task 2.
    let thief: u32 = (1..=3u32)
        .find(|id| *id != original_owner_of_2 && deployer.cluster.is_alive(*id))
        .expect("a free node must exist");

    let res = deployer
        .cluster
        .submit(
            deployer.cluster.leader().await.unwrap(),
            Op::StealTask {
                thief_node: thief,
                task_id: 2,
            },
        )
        .await;
    assert!(res.is_ok(), "StealTask submit itself must succeed (Raft appends before applying)");

    // Wait for the CP to reflect the new owner and status.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut won = false;
    while tokio::time::Instant::now() < deadline {
        let status = read_task_status_anywhere(&deployer.cluster, 2).await;
        let owner = read_task_owner_anywhere(&deployer.cluster, 2).await;
        if status == Some(TaskStatus::Migrating) && owner == Some(thief) {
            won = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        won,
        "task 2 must reach Migrating with new owner = {thief} within 2s of StealTask"
    );

    hb.stop();
    hb.join().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_steal_task_from_two_thieves_only_one_wins() {
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let tasks = vec![started(1), started(2), started(3)];
    let job_id = deployer
        .deploy(linear_pipeline(tasks, vec![]))
        .await
        .unwrap();

    let mapping = deployer.job_to_node_mapping(job_id).await;
    eprintln!("[ws-concurrent] task mapping: {mapping:?}");
    let original_owner_of_3 = mapping[&3];
    eprintln!("[ws-concurrent] original_owner_of_3={original_owner_of_3}");

    // Get into Orphaned state for task 3.
    let cluster = deployer.cluster.clone();
    let log = deployer.log.clone();
    let mut hb = HeartbeatOrchestrator::new(
        cluster,
        HeartbeatConfig {
            interval: Duration::from_millis(100),
            orphan_threshold: Duration::from_millis(300),
        },
        log,
    );
    hb.start();

    tokio::time::sleep(Duration::from_millis(400)).await;
    deployer.cluster.shutdown_node(original_owner_of_3).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(TaskStatus::Orphaned) = read_task_status_anywhere(&deployer.cluster, 3).await {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("task 3 did not become Orphaned within 2s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Two thieves (both alive nodes that aren't the original owner).
    let thieves: Vec<u32> = (1..=3u32)
        .filter(|id| *id != original_owner_of_3 && deployer.cluster.is_alive(*id))
        .collect();
    assert_eq!(thieves.len(), 2, "exactly 2 thieves expected");

    let leader = deployer.cluster.leader().await.unwrap();
    // Submit both StealTasks.
    for &t in &thieves {
        let _ = deployer
            .cluster
            .submit(
                leader,
                Op::StealTask {
                    thief_node: t,
                    task_id: 3,
                },
            )
            .await;
    }

    // Wait for the CP to converge.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut winner: Option<u32> = None;
    while tokio::time::Instant::now() < deadline {
        let status = read_task_status_anywhere(&deployer.cluster, 3).await;
        let owner = read_task_owner_anywhere(&deployer.cluster, 3).await;
        if status == Some(TaskStatus::Migrating) && owner.is_some() {
            // Find which thief won (or whether it was a non-thief somehow).
            if thieves.contains(&owner.unwrap()) {
                winner = owner;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let winner = winner.expect("exactly one thief must win within 2s");
    // The losing thief's StealTask was a no-op (the task is no longer
    // Orphaned). Verify by checking that the task owner is the winner,
    // not the loser.
    let final_owner = read_task_owner_anywhere(&deployer.cluster, 3).await;
    assert_eq!(final_owner, Some(winner), "final owner must be the winner");
    assert!(
        thieves.contains(&winner),
        "winner must be one of the thieves"
    );
    let other_thief = thieves.iter().find(|t| **t != winner).copied().unwrap();
    assert_ne!(final_owner, Some(other_thief), "loser must not own the task");

    hb.stop();
    hb.join().await;
}
