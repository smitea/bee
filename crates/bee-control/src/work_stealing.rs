//! `NodeThiefLoop` — per-Node background task that takes
//! ownership of `Orphaned` Tasks (S12).
//!
//! After the S11 leader marks a dead node's tasks as
//! `Orphaned` (via `Op::MarkNodeOrphaned`), each alive Node's
//! thief loop scans its local CP for `Orphaned` tasks not
//! owned by self, and submits `Op::StealTask { thief_node:
//! self }` to take ownership. The SM's atomic check-and-set
//! ensures only one thief wins; the others get a no-op (the
//! Task is already `Migrating`).
//!
//! KV Checkpoint (the third S12 acceptance criterion) is a
//! S12.x follow-up — too big for this quick win.

use std::time::Duration;

use crate::kv::{Op, TaskStatus};
use crate::raft::Cluster;

/// How often the thief loop scans the local CP for Orphaned
/// tasks. Per S12 design.
pub const THIEF_LOOP_INTERVAL: Duration = Duration::from_secs(1);

/// Spawn the per-Node thief loop as a background tokio task.
/// Returns the `JoinHandle` so the caller can shut it down
/// cleanly.
pub fn spawn_thief_loop(cluster: Cluster) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { run_thief_loop(cluster).await })
}

/// The thief loop itself. Polls the local CP every
/// `THIEF_LOOP_INTERVAL` and submits `StealTask` for every
/// `Orphaned` task not owned by this node.
///
/// In the in-process 3-Node cluster, all alive nodes share
/// the same CP (the leader's `ControlPlaneStateMachine`); so
/// we scan the leader's CP and treat the leader as the thief
/// for MVP. Cross-Node StealTask over BRP is a S33.1 follow-up.
async fn run_thief_loop(cluster: Cluster) {
    let mut ticker = tokio::time::interval(THIEF_LOOP_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if let Err(e) = try_steal_round(&cluster).await {
            eprintln!("thief loop: error during round: {e}");
        }
    }
}

/// One iteration of the thief loop: scan the leader's CP for
/// `Orphaned` tasks, and submit `StealTask { thief_node:
/// leader }` for each.
async fn try_steal_round(cluster: &Cluster) -> Result<(), String> {
    let Some(leader) = cluster.leader().await else {
        // No leader yet — will retry next tick.
        return Ok(());
    };
    let leader_handle = cluster
        .nodes()
        .find(|(id, _)| *id == leader)
        .map(|(_, h)| h)
        .ok_or_else(|| format!("leader node {leader} handle not found"))?
        .clone();
    let cp = leader_handle.cp.lock().await;
    // Collect Orphaned task ids first to avoid holding the
    // lock across the await on `submit`.
    let candidates: Vec<u32> = cp
        .list_tasks()
        .iter()
        .filter(|t| t.status == TaskStatus::Orphaned)
        .map(|t| t.task_id)
        .collect();
    drop(cp);

    for task_id in candidates {
        let resp = cluster
            .submit(
                leader,
                Op::StealTask {
                    thief_node: leader,
                    task_id,
                },
            )
            .await
            .map_err(|e| format!("submit StealTask({task_id}): {e}"))?;
        eprintln!(
            "thief loop: StealTask(task={task_id}, thief={leader}) -> {resp:?}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raft::ClusterConfig;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn try_steal_round_no_op_when_no_orphans() {
        // Sanity check: a fresh cluster has no Orphaned tasks;
        // the loop should be a no-op (no StealTask submissions).
        let cluster = Cluster::new(ClusterConfig::default()).await;
        cluster
            .wait_for_leader(std::time::Duration::from_secs(3))
            .await
            .expect("leader elected");
        try_steal_round(&cluster).await.expect("no-op round");
    }
}
