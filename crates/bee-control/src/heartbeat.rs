//! S11 Heartbeat + Failover detection.
//!
//! Each non-Leader Node submits `Op::Heartbeat { node_id, timestamp_ms }`
//! to the Cluster every `interval`. The Leader reads its own
//! `ControlPlaneStateMachine::last_heartbeat` to find nodes that have
//! missed `orphan_threshold` worth of heartbeats, and for each such
//! node submits `Op::UpdateTaskStatus { .., Orphaned }` for every Task
//! it owns.
//!
//! ## "High-priority dedicated channel" (ADR-0007)
//! Heartbeat is a ControlPlane op that goes through the Raft log — it is
//! a separate, Raft-replicated stream from the BRP data channel that
//! carries Phase-to-Phase events (S09 in-process: `mpsc` forwarders).
//! In a multi-process deployment, the two channels would also be
//! physically separate TCP connections (control vs data) per ADR-0007.
//!
//! ## Configurability
//! `HeartbeatConfig { interval, orphan_threshold }` is configurable. The
//! spec's defaults (10s interval, 30s threshold) make the integration test
//! slow; tests override to short values.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::builtin_handlers::LogSink;
use crate::kv::{Op, TaskStatus};
use crate::raft::cluster::Cluster;

#[derive(Clone, Debug)]
pub struct HeartbeatConfig {
    pub interval: Duration,
    pub orphan_threshold: Duration,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(10),
            orphan_threshold: Duration::from_secs(30),
        }
    }
}

pub struct HeartbeatOrchestrator {
    pub cluster: Cluster,
    pub config: HeartbeatConfig,
    pub log: LogSink,
    handle: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl HeartbeatOrchestrator {
    pub fn new(cluster: Cluster, config: HeartbeatConfig, log: LogSink) -> Self {
        Self {
            cluster,
            config,
            log,
            handle: None,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(&mut self) {
        self.running.store(true, Ordering::SeqCst);
        let cluster = self.cluster.clone();
        let config = self.config.clone();
        let log = self.log.clone();
        let running = self.running.clone();
        let handle = tokio::spawn(async move {
            run_heartbeat_loop(cluster, config, log).await;
            running.store(false, Ordering::SeqCst);
        });
        self.handle = Some(handle);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }

    pub async fn join(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.await;
        }
    }
}

async fn run_heartbeat_loop(cluster: Cluster, config: HeartbeatConfig, log: LogSink) {
    let mut ticker = tokio::time::interval_at(
        Instant::now() + config.interval,
        config.interval,
    );
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;

        let Some(leader) = cluster.leader().await else {
            continue;
        };

        let now_ms = std_time_ms();
        let node_ids: Vec<u32> = node_ids_in_cluster(&cluster);
        for node_id in node_ids {
            if node_id == leader {
                detect_and_mark_orphans(&cluster, &config, now_ms, &log).await;
            } else if !cluster.is_alive(node_id) {
                // Dead node: don't bother sending its heartbeat.
                // The leader will mark its tasks Orphaned once its
                // last_heartbeat goes stale.
                continue;
            } else {
                let _ = cluster
                    .submit(
                        leader,
                        Op::Heartbeat {
                            node_id,
                            timestamp_ms: now_ms,
                        },
                    )
                    .await;
            }
        }
    }
}

fn node_ids_in_cluster(cluster: &Cluster) -> Vec<u32> {
    cluster.nodes().map(|(id, _)| id).collect()
}

async fn detect_and_mark_orphans(
    cluster: &Cluster,
    config: &HeartbeatConfig,
    now_ms: u64,
    log: &LogSink,
) {
    let Some(leader) = cluster.leader().await else {
        return;
    };
    let Some(leader_node) = cluster.node(leader) else {
        return;
    };
    let threshold_ms = config.orphan_threshold.as_millis() as u64;

    let stale_nodes: Vec<u32> = {
        let cp = leader_node.cp.lock().await;
        cp.stale_nodes(now_ms, threshold_ms)
    };

    for stale in stale_nodes {
        let tasks: Vec<u32> = {
            let cp = leader_node.cp.lock().await;
            cp.tasks_owned_by(stale)
                .into_iter()
                .map(|t| t.task_id)
                .collect()
        };
        for task_id in tasks {
            log.record(format!(
                "orphan: node {stale} silent > {:?}, marking task {task_id} as Orphaned",
                config.orphan_threshold
            ));
            let _ = cluster
                .submit(
                    leader,
                    Op::UpdateTaskStatus {
                        task_id,
                        new_status: TaskStatus::Orphaned,
                    },
                )
                .await;
        }
    }
}

fn std_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
