//! `bee-control` — Bee 控制面。
//!
//! 封装 Raft 客户端、Scheduler (bin-packing / MLFQ)、Work-Stealing 仲裁器。
//! 所有"谁拥有某个 Task"的变更必须经 Raft Leader 仲裁后落地。
//!
//! S00 阶段仅占位;S06 起实现单 Node Raft 循环 + KV state machine;
//! S07 起接 Raft 多节点 (in-process 3 节点 + 选举 + 日志复制);
//! S10 起实现 Scheduler;S11 起实现 StealArbiter。

pub mod builtin_handlers;
pub mod cluster_status;
pub mod control_plane;
pub mod deployer;
pub mod diagnostics_view;
pub mod datasource;
pub mod heartbeat;
pub mod jobs_view;
pub mod kv;
pub mod raft;
pub mod rebalancer;
pub mod scheduler;
pub mod secret_store;
pub mod signature;
pub mod worker;
pub mod work_stealing;

// S-1c: test-only harness (3-node Cluster + per-node AdminServer).
// Gated so production binaries do not pull in the wiring. Tests
// inside this crate pick it up via `cfg(test)`; downstream test
// crates (e.g. bee-gui) opt in via the `test-utils` feature in
// their dev-dependencies.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub use bee_types::JobLifecycleState;

pub use builtin_handlers::{LogSink, StartedHandler, TerminalHandler};
pub use control_plane::{ControlPlaneStateMachine, DependencyRecord, JobRecord, TaskRecord};
pub use deployer::{Deployer, DeployerConfig, Edge, HandlerKind, Pipeline, TaskSpec};
pub use heartbeat::{HeartbeatConfig, HeartbeatOrchestrator};
pub use kv::{KVStateMachine, Op, TaskStatus, TxnError};
pub use raft::{
    Cluster, ClusterConfig, LogIndex, NodeCommand, NodeId, NodeMetrics, RpcMessage, Term,
};
pub use scheduler::{
    FirstFitDecreasingScheduler, NodeCapacity, Scheduler, TaskPlacement, TaskRequirement,
};
pub use worker::{TaskWorker, WorkerCapacity};
