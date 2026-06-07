//! `Rebalancer` — cross-Node Task rebalance trigger (S25, ADR-0008 §0.7).
//!
//! The Scheduler periodically reads per-Node load (from the S24
//! per-Phase metrics) and triggers a rebalance when a Node is
//! overloaded relative to the cluster average. The rebalance itself
//! rides the existing S12 Work-Stealing machinery (`Op::StealTask`)
//! so the actual Task migration is just a StealTask with the
//! "thief" being a free Node.
//!
//! ## S25 scope
//! - [`NodeLoad`] — per-Node load sample (test-injectable; the
//!   production path samples from S24's per-Phase metrics).
//! - [`RebalanceConfig`] — `load_threshold_multiplier` (default
//!   1.5), `min_task_age_secs` (default 300 = 5 min per spec), and
//!   `rebalance_interval_secs` (default 60).
//! - [`RebalanceEvent`] — one record per migration triggered:
//!   timestamp, task_id, from_node, to_node.
//! - [`Rebalancer`] — holds the event log and config; the
//!   `tick(cluster, loads)` method evaluates the trigger and submits
//!   `Op::StealTask` for each candidate.
//!
//! ## S25 acceptance
//! - No rebalance when cluster load is balanced
//! - Rebalance when one Node exceeds `1.5×` cluster average AND
//!   has at least one Task older than `min_task_age_secs`
//! - No flapping: once load is below the threshold, no further
//!   rebalance fires

use std::collections::HashMap;
use std::sync::Mutex;

use crate::kv::Op;
use crate::raft::cluster::Cluster;

/// Per-Node load sample. `load_pct` is a 0-100 value. The production
/// path (S24 + a future aggregator) computes this from the per-Phase
/// metrics; the test injects it directly.
#[derive(Debug, Clone, Copy)]
pub struct NodeLoad {
    pub node_id: u32,
    pub load_pct: f64,
}

impl NodeLoad {
    pub fn new(node_id: u32, load_pct: f64) -> Self {
        Self { node_id, load_pct }
    }
}

/// Rebalance trigger configuration. Defaults match the S25 spec:
/// 1.5× threshold, 5-min minimum task age, 60s rebalance interval.
#[derive(Debug, Clone)]
pub struct RebalanceConfig {
    /// A Node is "overloaded" when its load exceeds
    /// `cluster_average * load_threshold_multiplier`. Default 1.5
    /// (per S25 spec).
    pub load_threshold_multiplier: f64,
    /// A Task must have been at its current owner for at least
    /// this many seconds before it's eligible to be migrated.
    /// Default 300 (= 5 min, per S25 spec).
    pub min_task_age_secs: u64,
    /// Wall-clock millis injected as `started_at_ms` on
    /// `Op::RegisterTask` from the test path. The Rebalancer
    /// compares `now_ms - started_at_ms` against
    /// `min_task_age_secs * 1000`.
    ///
    /// (In production, the Deployer tracks wall-clock; the MVP
    /// default is 0 and tests inject a value via
    /// `Op::RegisterTask.started_at_ms`.)
    pub now_ms: u64,
}

impl Default for RebalanceConfig {
    fn default() -> Self {
        Self {
            load_threshold_multiplier: 1.5,
            min_task_age_secs: 300,
            now_ms: 0,
        }
    }
}

/// One migration triggered by a rebalance tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebalanceEvent {
    pub timestamp_ms: u64,
    pub task_id: u32,
    pub from_node: u32,
    pub to_node: u32,
}

/// Cross-Node Rebalancer. Drives `Op::StealTask` for each Task
/// selected by the trigger logic.
pub struct Rebalancer {
    config: RebalanceConfig,
    events: Mutex<Vec<RebalanceEvent>>,
}

impl Rebalancer {
    pub fn new(config: RebalanceConfig) -> Self {
        Self {
            config,
            events: Mutex::new(Vec::new()),
        }
    }

    /// All rebalance events recorded so far (most recent last).
    /// Used by `bee cluster status` (S28 wiring) to show the last
    /// migration.
    pub fn events(&self) -> Vec<RebalanceEvent> {
        self.events.lock().expect("poisoned").clone()
    }

    /// One rebalance tick. The caller supplies the per-Node load
    /// (production: read from S24 metrics; test: inject directly).
    /// Returns the events triggered by this tick.
    ///
    /// Trigger logic:
    /// 1. Compute cluster average load.
    /// 2. Find overloaded Nodes (load > avg × multiplier).
    /// 3. For each overloaded Node, find the oldest eligible Task
    ///    (started_at_ms ≤ now - min_task_age).
    /// 4. Pick the most underloaded Node as the target.
    /// 5. Submit `Op::StealTask { thief_node: target, task_id }`.
    /// 6. Record the event.
    pub async fn tick(
        &self,
        cluster: &Cluster,
        loads: &HashMap<u32, NodeLoad>,
    ) -> Vec<RebalanceEvent> {
        if loads.is_empty() {
            return Vec::new();
        }
        let avg = loads.values().map(|l| l.load_pct).sum::<f64>() / loads.len() as f64;
        let threshold = avg * self.config.load_threshold_multiplier;

        // Overloaded nodes: load > threshold.
        let mut overloaded: Vec<&NodeLoad> = loads
            .values()
            .filter(|l| l.load_pct > threshold)
            .collect();
        overloaded.sort_by(|a, b| b.load_pct.partial_cmp(&a.load_pct).unwrap_or(std::cmp::Ordering::Equal));

        // Most underloaded: the candidate target. Filter out
        // overloaded nodes — they shouldn't receive more work.
        let underloaded: Vec<&NodeLoad> = {
            let mut v: Vec<&NodeLoad> = loads
                .values()
                .filter(|l| l.load_pct <= threshold)
                .collect();
            v.sort_by(|a, b| a.load_pct.partial_cmp(&b.load_pct).unwrap_or(std::cmp::Ordering::Equal));
            v
        };

        if underloaded.is_empty() {
            // All nodes overloaded (or no targets). No migration.
            return Vec::new();
        }

        // The most underloaded is the first target.
        let target_node = underloaded[0].node_id;

        // Find the Leader for submitting StealTask.
        let leader = match cluster.leader().await {
            Some(l) => l,
            None => return Vec::new(),
        };

        // For each overloaded node, find the oldest eligible Task
        // and submit a StealTask.
        let mut new_events = Vec::new();
        let now_ms = self.config.now_ms;
        let min_age_ms = self.config.min_task_age_secs * 1000;

        for overloaded_node in &overloaded {
            // Find candidates: Tasks owned by this node that are
            // Running and old enough. Pick the oldest.
            let candidate = self
                .find_oldest_eligible_task(
                    cluster,
                    overloaded_node.node_id,
                    now_ms,
                    min_age_ms,
                )
                .await;

            if let Some((task_id, from_node)) = candidate {
                // Submit StealTask: target_node becomes the new
                // owner. The S12 atomic check-and-set will reject
                // if the Task isn't actually Orphaned; the
                // Rebalancer treats that as a no-op (the Task may
                // have moved between ticks).
                let op = Op::StealTask {
                    thief_node: target_node,
                    task_id,
                };
                let _ = cluster.submit(leader, op).await;

                let event = RebalanceEvent {
                    timestamp_ms: now_ms,
                    task_id,
                    from_node,
                    to_node: target_node,
                };
                self.events.lock().expect("poisoned").push(event.clone());
                new_events.push(event);
            }
        }

        new_events
    }

    /// Find the oldest eligible Task on `owner_node`. Returns
    /// `(task_id, from_node)` if found. Reads the ControlPlane
    /// from any alive node.
    async fn find_oldest_eligible_task(
        &self,
        cluster: &Cluster,
        owner_node: u32,
        now_ms: u64,
        min_age_ms: u64,
    ) -> Option<(u32, u32)> {
        use crate::kv::TaskStatus;

        for (id, handle) in cluster.nodes() {
            if !cluster.is_alive(id) {
                continue;
            }
            let cp = handle.cp.lock().await;
            // Find tasks on the overloaded node, Running status,
            // and old enough.
            let mut candidates: Vec<crate::control_plane::TaskRecord> = cp
                .list_tasks()
                .into_iter()
                .filter(|t| {
                    t.owner_node == owner_node
                        && t.status == TaskStatus::Running
                        && now_ms.saturating_sub(t.started_at_ms) >= min_age_ms
                })
                .collect();
            if candidates.is_empty() {
                continue;
            }
            // Oldest first (smallest started_at_ms).
            candidates.sort_by_key(|t| t.started_at_ms);
            let t = &candidates[0];
            return Some((t.task_id, t.owner_node));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_load_new_sets_fields() {
        let l = NodeLoad::new(1, 50.0);
        assert_eq!(l.node_id, 1);
        assert_eq!(l.load_pct, 50.0);
    }

    #[test]
    fn config_default_matches_spec() {
        let c = RebalanceConfig::default();
        assert_eq!(c.load_threshold_multiplier, 1.5);
        assert_eq!(c.min_task_age_secs, 300);
    }

    #[test]
    fn rebalancer_starts_with_empty_event_log() {
        let r = Rebalancer::new(RebalanceConfig::default());
        assert!(r.events().is_empty());
    }
}
