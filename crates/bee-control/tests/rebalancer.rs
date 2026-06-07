//! S25: Cross-Node rebalance — trigger conditions + event log.
//!
//! The Rebalancer is the S25 surface that reads per-Node load
//! (test-injected) and submits `Op::StealTask` to migrate eligible
//! Tasks. The tests below cover the trigger conditions and the
//! no-flapping guarantee.

use std::collections::HashMap;
use std::time::Duration;

use bee_control::kv::{Op, TaskStatus};
use bee_control::raft::cluster::{Cluster, ClusterConfig};
use bee_control::rebalancer::{NodeLoad, RebalanceConfig, Rebalancer};

async fn fresh_cluster() -> Cluster {
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader elected");
    cluster
}

/// Register N tasks owned by `owner_node` with `started_at_ms` set
/// so they're eligible for migration. `started_at_ms` is set to
/// `now_ms - 600_000` (10 min in the past) by default, so even the
/// 5-min `min_task_age_secs` rule passes.
async fn register_old_task(
    cluster: &Cluster,
    task_id: u32,
    owner_node: u32,
    started_at_ms: u64,
) {
    let leader = cluster.leader().await.expect("leader");
    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: task_id,
                dag_hash: "h".into(),
                owner_node,
            },
        )
        .await
        .unwrap();
    cluster
        .submit(
            leader,
            Op::RegisterTask {
                task_id,
                job_id: task_id,
                phase_id: 0,
                owner_node,
                status: TaskStatus::Running,
                started_at_ms,
            },
        )
        .await
        .unwrap();
}

async fn wait_for_apply(_cluster: &Cluster, timeout: Duration) {
    // Give the Raft log a beat to apply.
    tokio::time::sleep(timeout).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_rebalance_when_load_is_balanced() {
    let cluster = fresh_cluster().await;
    let rebalancer = Rebalancer::new(RebalanceConfig::default());

    // 3 nodes, equal load → no overload, no migration.
    let mut loads = HashMap::new();
    loads.insert(1, NodeLoad::new(1, 30.0));
    loads.insert(2, NodeLoad::new(2, 30.0));
    loads.insert(3, NodeLoad::new(3, 30.0));
    let events = rebalancer.tick(&cluster, &loads).await;
    assert!(events.is_empty(), "balanced cluster should not rebalance, got {events:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rebalance_triggers_when_one_node_exceeds_threshold() {
    // 3 nodes: node 1 at 80% (overloaded), nodes 2 + 3 at 10%
    // each. Cluster average = 33.3%. 1.5× threshold = 50%.
    // 80% > 50% → node 1 is overloaded. Target = most underloaded
    // = node 2 (10%).
    let cluster = fresh_cluster().await;
    let cfg = RebalanceConfig {
        load_threshold_multiplier: 1.5,
        min_task_age_secs: 0, // accept all tasks in the test
        now_ms: 1_000_000_000, // arbitrary fixed clock for the test
    };
    let rebalancer = Rebalancer::new(cfg);

    // Register an old task on node 1.
    register_old_task(&cluster, 1, 1, 0).await; // started_at = 0, age = 1e9
    wait_for_apply(&cluster, Duration::from_millis(200)).await;

    let mut loads = HashMap::new();
    loads.insert(1, NodeLoad::new(1, 80.0));
    loads.insert(2, NodeLoad::new(2, 5.0));
    loads.insert(3, NodeLoad::new(3, 15.0));

    let events = rebalancer.tick(&cluster, &loads).await;
    assert_eq!(events.len(), 1, "expected one migration, got {events:?}");
    let e = &events[0];
    assert_eq!(e.task_id, 1);
    assert_eq!(e.from_node, 1, "task was on node 1");
    assert_eq!(e.to_node, 2, "target is most underloaded (node 2 at 5%)");
    assert_eq!(e.timestamp_ms, 1_000_000_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_rebalance_for_young_tasks() {
    // Node 1 is overloaded but its only task just started (age
    // < min_task_age_secs). No migration should fire.
    let cluster = fresh_cluster().await;
    let now_ms = 1_000_000_000u64;
    let cfg = RebalanceConfig {
        load_threshold_multiplier: 1.5,
        min_task_age_secs: 300,
        now_ms,
    };
    let rebalancer = Rebalancer::new(cfg);

    // Task started 10s ago — well within the 5-min gate.
    register_old_task(&cluster, 1, 1, now_ms - 10_000).await;
    wait_for_apply(&cluster, Duration::from_millis(200)).await;

    let mut loads = HashMap::new();
    loads.insert(1, NodeLoad::new(1, 80.0));
    loads.insert(2, NodeLoad::new(2, 10.0));
    loads.insert(3, NodeLoad::new(3, 10.0));
    let events = rebalancer.tick(&cluster, &loads).await;
    assert!(
        events.is_empty(),
        "young tasks must NOT be migrated, got {events:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_task_above_min_age_does_trigger_rebalance() {
    // Symmetric to the previous test: task started 10 min ago, the
    // 5-min gate passes, rebalance fires.
    let cluster = fresh_cluster().await;
    let now_ms = 1_000_000_000u64;
    let cfg = RebalanceConfig {
        load_threshold_multiplier: 1.5,
        min_task_age_secs: 300,
        now_ms,
    };
    let rebalancer = Rebalancer::new(cfg);

    register_old_task(&cluster, 1, 1, now_ms - 600_000).await; // 10 min ago
    wait_for_apply(&cluster, Duration::from_millis(200)).await;

    let mut loads = HashMap::new();
    loads.insert(1, NodeLoad::new(1, 80.0));
    loads.insert(2, NodeLoad::new(2, 10.0));
    loads.insert(3, NodeLoad::new(3, 10.0));
    let events = rebalancer.tick(&cluster, &loads).await;
    assert_eq!(events.len(), 1, "old task should migrate, got {events:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_flapping_after_load_normalizes() {
    // S25 acceptance: if a Node's load drops back to normal, no
    // rebalance fires. The trigger has a hard cutoff: once load is
    // below `avg × 1.5`, no second migration is issued.
    let cluster = fresh_cluster().await;
    let cfg = RebalanceConfig {
        load_threshold_multiplier: 1.5,
        min_task_age_secs: 0,
        now_ms: 1_000_000_000,
    };
    let rebalancer = Rebalancer::new(cfg);

    // Tick 1: 80% / 10% / 10% → trigger.
    register_old_task(&cluster, 1, 1, 0).await;
    wait_for_apply(&cluster, Duration::from_millis(200)).await;

    let mut loads = HashMap::new();
    loads.insert(1, NodeLoad::new(1, 80.0));
    loads.insert(2, NodeLoad::new(2, 10.0));
    loads.insert(3, NodeLoad::new(3, 10.0));
    let events_1 = rebalancer.tick(&cluster, &loads).await;
    assert_eq!(events_1.len(), 1, "first tick should rebalance");

    // Tick 2: load normalizes to 40% / 40% / 40% → no overload.
    let mut loads = HashMap::new();
    loads.insert(1, NodeLoad::new(1, 40.0));
    loads.insert(2, NodeLoad::new(2, 40.0));
    loads.insert(3, NodeLoad::new(3, 40.0));
    let events_2 = rebalancer.tick(&cluster, &loads).await;
    assert!(
        events_2.is_empty(),
        "second tick should be a no-op, got {events_2:?}"
    );

    // The event log only has the first migration.
    let all = rebalancer.events();
    assert_eq!(all.len(), 1, "only the first rebalance was recorded");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picks_most_underloaded_node_as_target() {
    // Among the underloaded nodes, the one with the lowest load is
    // the target.
    let cluster = fresh_cluster().await;
    let cfg = RebalanceConfig {
        load_threshold_multiplier: 1.5,
        min_task_age_secs: 0,
        now_ms: 1_000_000_000,
    };
    let rebalancer = Rebalancer::new(cfg);
    register_old_task(&cluster, 1, 1, 0).await;
    wait_for_apply(&cluster, Duration::from_millis(200)).await;

    let mut loads = HashMap::new();
    loads.insert(1, NodeLoad::new(1, 80.0));
    loads.insert(2, NodeLoad::new(2, 30.0));
    loads.insert(3, NodeLoad::new(3, 5.0)); // most underloaded
    let events = rebalancer.tick(&cluster, &loads).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].to_node, 3, "most underloaded is node 3");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rebalance_event_recorded_in_log() {
    // The event log accumulates across ticks (no flapping, no
    // re-recording).
    let cluster = fresh_cluster().await;
    let cfg = RebalanceConfig {
        load_threshold_multiplier: 1.5,
        min_task_age_secs: 0,
        now_ms: 1_000_000_000,
    };
    let rebalancer = Rebalancer::new(cfg);
    register_old_task(&cluster, 1, 1, 0).await;
    register_old_task(&cluster, 2, 1, 0).await;
    wait_for_apply(&cluster, Duration::from_millis(200)).await;

    let mut loads = HashMap::new();
    loads.insert(1, NodeLoad::new(1, 80.0));
    loads.insert(2, NodeLoad::new(2, 5.0));
    loads.insert(3, NodeLoad::new(3, 10.0));
    let events = rebalancer.tick(&cluster, &loads).await;
    // Both tasks on node 1 are eligible (old enough). The
    // Rebalancer iterates overloaded nodes and picks the oldest
    // task per node; with both on node 1, only one migration per
    // tick fires.
    assert_eq!(events.len(), 1);
    let all = rebalancer.events();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].from_node, 1);
    assert_eq!(all[0].to_node, 2, "node 2 is most underloaded (5%)");
}
