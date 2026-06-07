//! `bee-control` — Bee 控制面。
//!
//! 封装 Raft 客户端、Scheduler (bin-packing / MLFQ)、Work-Stealing 仲裁器。
//! 所有"谁拥有某个 Task"的变更必须经 Raft Leader 仲裁后落地。
//!
//! S00 阶段仅占位;S06 起实现单 Node Raft 循环 + KV state machine;
//! S07 起接 Raft 多节点 (in-process 3 节点 + 选举 + 日志复制);
//! S10 起实现 Scheduler;S11 起实现 StealArbiter。

pub mod builtin_handlers;
pub mod control_plane;
pub mod deployer;
pub mod heartbeat;
pub mod kv;
pub mod raft;
pub mod scheduler;
pub mod worker;

pub use builtin_handlers::{LogSink, StartedHandler, TerminalHandler};
pub use control_plane::{ControlPlaneStateMachine, JobRecord, TaskRecord};
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
