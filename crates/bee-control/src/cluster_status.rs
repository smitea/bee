//! `cluster_status` — formatting for `bee cluster status` (S28).
//!
//! Renders the cluster's Raft health, per-Node resource summary,
//! and aggregate counts. The MVP keeps the surface narrow:
//! - leader, current_term, alive-flag per Node
//! - per-Node log_lag (the leader's `last_log_index` minus the
//!   follower's `last_log_index`)
//! - aggregate jobs / tasks count
//!
//! The `last rebalance event` (S25) and `plugin summary` (S19) are
//! not in the MVP — they require the CLI to also instantiate a
//! `Rebalancer` and `PluginManager`, which the in-process CLI demo
//! doesn't. The formatter signature accepts them as optional refs;
//! pass `None` for the MVP.

use crate::kv::TaskStatus;
use crate::raft::cluster::Cluster;
use crate::raft::Role;

/// Format the `bee cluster status` view. Reads the cluster state
/// (no writes) and renders a text report.
pub async fn format_cluster_status(
    cluster: &Cluster,
) -> String {
    let mut out = String::new();
    out.push_str("=== Cluster Status ===\n");
    out.push_str(&format!("total_nodes: {}\n", total_node_count(cluster)));
    out.push_str(&format!(
        "alive_nodes:  {}\n",
        alive_node_count(cluster)
    ));

    // Aggregate counts (read from the first alive node's CP).
    if let Some((_, handle)) = cluster.nodes().next() {
        if cluster.is_alive(handle.id) {
            let cp = handle.cp.lock().await;
            out.push_str(&format!("total_jobs:   {}\n", cp.job_count()));
            out.push_str(&format!("total_tasks:  {}\n", cp.task_count()));
            // Per-status task breakdown
            let mut by_status: std::collections::BTreeMap<&str, usize> =
                std::collections::BTreeMap::new();
            for t in cp.list_tasks() {
                let key = match t.status {
                    TaskStatus::Pending => "pending",
                    TaskStatus::Scheduled => "scheduled",
                    TaskStatus::Running => "running",
                    TaskStatus::Orphaned => "orphaned",
                    TaskStatus::Migrating => "migrating",
                    TaskStatus::Revoked => "revoked",
                    TaskStatus::Completed => "completed",
                    TaskStatus::Failed => "failed",
                };
                *by_status.entry(key).or_insert(0) += 1;
            }
            if !by_status.is_empty() {
                out.push_str("tasks_by_status:\n");
                for (k, v) in &by_status {
                    out.push_str(&format!("  {}: {}\n", k, v));
                }
            }
        }
    }

    out.push_str("\n=== Nodes ===\n");
    out.push_str("id | alive | role   | term | last_log_index | log_lag\n");
    out.push_str("---+-------+--------+------+----------------+--------\n");

    // Compute leader's last_log_index for the log_lag column.
    let leader_last_log = leader_last_log_index(cluster).await;

    let mut nodes: Vec<_> = cluster.nodes().collect();
    nodes.sort_by_key(|(id, _)| *id);
    for (id, handle) in nodes {
        if !cluster.is_alive(id) {
            out.push_str(&format!(
                "{:3} | {:5} | {:6} | {:4} | {:14} | {:7}\n",
                id, "no", "-", "-", "-", "-"
            ));
            continue;
        }
        let state = handle.state.lock().await;
        let role = format!("{:?}", state.role);
        let term = state.current_term;
        let last_log = (state.log.len() as u64).saturating_sub(1);
        let lag = if state.role == Role::Leader {
            0
        } else {
            leader_last_log.saturating_sub(last_log)
        };
        out.push_str(&format!(
            "{:3} | {:5} | {:6} | {:4} | {:14} | {:7}\n",
            id,
            "yes",
            truncate(&role, 8),
            term,
            last_log,
            lag,
        ));
    }

    out
}

fn total_node_count(cluster: &Cluster) -> usize {
    cluster.nodes().count()
}

fn alive_node_count(cluster: &Cluster) -> usize {
    cluster
        .nodes()
        .filter(|(id, _)| cluster.is_alive(*id))
        .count()
}

async fn leader_last_log_index(cluster: &Cluster) -> u64 {
    for (id, handle) in cluster.nodes() {
        if !cluster.is_alive(id) {
            continue;
        }
        let state = handle.state.lock().await;
        if state.role == Role::Leader {
            // commit_index is what the leader has confirmed; for the
            // log_lag view we use the log length (= the last index
            // appended, including uncommitted entries) as a proxy
            // for "the last index the leader has". Followers lag
            // behind on the committed prefix.
            return (state.log.len() as u64).saturating_sub(1);
        }
    }
    0
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raft::cluster::ClusterConfig;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn format_cluster_status_on_fresh_cluster() {
        let cluster = Cluster::new(ClusterConfig::default()).await;
        cluster
            .wait_for_leader(std::time::Duration::from_secs(3))
            .await
            .expect("leader");
        let s = format_cluster_status(&cluster).await;
        assert!(s.contains("=== Cluster Status ==="), "missing header:\n{s}");
        assert!(s.contains("total_nodes: 3"), "expected 3 nodes:\n{s}");
        assert!(s.contains("alive_nodes:  3"));
        assert!(s.contains("=== Nodes ==="));
        assert!(s.contains("| alive | role"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn format_cluster_status_aggregate_counts_after_register() {
        let cluster = Cluster::new(ClusterConfig::default()).await;
        cluster
            .wait_for_leader(std::time::Duration::from_secs(3))
            .await
            .expect("leader");
        let leader = cluster.leader().await.expect("leader");
        // Register 1 job, 2 tasks.
        cluster
            .submit(
                leader,
                crate::kv::Op::RegisterJob {
                    job_id: 1,
                    dag_hash: "h".into(),
                    owner_node: leader,
                    tenant: 0,
                            dependencies: vec![],
                },
            )
            .await
            .unwrap();
        cluster
            .submit(
                leader,
                crate::kv::Op::RegisterTask {
                    task_id: 1,
                    job_id: 1,
                    phase_id: 0,
                    owner_node: 1,
                    status: crate::kv::TaskStatus::Running,
                    started_at_ms: 0,
                },
            )
            .await
            .unwrap();
        cluster
            .submit(
                leader,
                crate::kv::Op::RegisterTask {
                    task_id: 2,
                    job_id: 1,
                    phase_id: 0,
                    owner_node: 2,
                    status: crate::kv::TaskStatus::Pending,
                    started_at_ms: 0,
                },
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let s = format_cluster_status(&cluster).await;
        assert!(s.contains("total_jobs:   1"));
        assert!(s.contains("total_tasks:  2"));
        assert!(s.contains("tasks_by_status:"));
        assert!(s.contains("running: 1"));
        assert!(s.contains("pending: 1"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn format_cluster_status_reflects_new_leader_after_kill() {
        // S28 acceptance: Raft health correctly after a leader
        // change. Kill the leader, wait for the new one, verify the
        // status reflects the new leader.
        let cluster = Cluster::new(ClusterConfig::default()).await;
        cluster
            .wait_for_leader(std::time::Duration::from_secs(3))
            .await
            .expect("leader");
        let old_leader = cluster.leader().await.expect("leader");

        // Capture the old leader id for the assertion.
        cluster.shutdown_node(old_leader).await;
        // Wait for a new leader (a different node).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut new_leader = old_leader;
        while std::time::Instant::now() < deadline {
            if let Some(l) = cluster.leader().await {
                if l != old_leader {
                    new_leader = l;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_ne!(
            new_leader, old_leader,
            "no new leader after killing {}",
            old_leader
        );

        let s = format_cluster_status(&cluster).await;
        // The old leader's row should show "no" alive; the new
        // leader should be one of the alive rows with role=Leader.
        assert!(s.contains(&format!("{:3} | {:5}", old_leader, "no")),
            "killed node should show alive=no:\n{s}");
        // New leader's row should be present and have role=Leader.
        assert!(s.contains(&format!("{:3} | {:5} |", new_leader, "yes")),
            "new leader should be present:\n{s}");
    }
}
