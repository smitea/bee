//! S33.2: the wire format for one tick of the
//! 24h soak monitoring loop. Each tick is
//! `bincode`-serialized and stored under
//! `soak/run_<RUN_ID>/tick_<TS_MS>` in the
//! leader's Raft KV. The human reads back via
//! `bee --connect <LEADER> kv list
//! soak/run_<RUN_ID>/` after the soak finishes.

use serde::{Deserialize, Serialize};

use super::admin_protocol::{
    ClusterMetricsDetail, JobSummary, TaskDiagDetail,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickMetrics {
    pub ts_unix_ms: u64,
    pub elapsed_sec: u64,
    pub cluster: ClusterMetricsDetail,
    pub jobs: Vec<JobSummary>,
    pub tasks: Vec<TaskDiagDetail>,
    pub influx_klines_per_min: u64,
    pub mongo_trades_per_min: u64,
    pub failover_at_ms: Option<u64>,
    pub recovered_at_ms: Option<u64>,
}
