//! Self-implemented minimal Raft for Bee control plane.
//!
//! ## S07 (this slice)
//! In-process 3-node cluster, no real network. Each node is a separate
//! `tokio` task; messages pass through `mpsc` channels (one inbox per node,
//! one outbox per peer). Election is timeout-driven with deterministic
//! per-node offsets so 2-node post-failure elections don't livelock.
//!
//! ## S07 spec coverage
//! - 3 separate nodes (or threads) form a Raft cluster — yes
//! - leader election, kill-leader recovery within 2s — yes
//! - log replication: Leader → Followers, commit on majority — yes
//! - apply to KV state machine after commit — yes
//! - BRP control channel carries Raft RPCs — deferred; S07 uses
//!   in-process mpsc; S07+ would replace the transport trait impl
//!   with a BRP-over-TCP version reusing bee-transport from S02
//!
//! ## Deferred
//! - Real BRP transport (S07+)
//! - Persistent log (S07+ would add a `LogStore`; S06's KV state
//!   machine is the in-memory apply target only)
//! - Snapshotting (1.x)
//! - Membership changes (1.x)

pub mod admin_protocol;
pub mod cluster;
pub mod node;
pub mod tcp;
pub mod transport;
pub mod types;

pub use admin_protocol::{
    AdminRequest, AdminResponse, ClusterMetricsDetail, JobDep, JobDetail, JobSummary,
    NodeMetricsSummary, TaskDiagDetail,
};
pub use cluster::{Cluster, ClusterConfig, ClusterNodeHandle, NodeMetrics, NodeSpec, NodeTransportSpec};
pub use node::{Node, NodeConfig, NodeState};
pub use tcp::TcpTransport;
pub use transport::{InMemoryTransport, NodeTransport, Router, TransportError};
pub use types::{LogEntry, NodeCommand, NodeId, RpcMessage, Role, Term, LogIndex};
