//! `bee-control` — Bee 控制面。
//!
//! 封装 Raft 客户端、Scheduler (bin-packing / MLFQ)、Work-Stealing 仲裁器。
//! 所有"谁拥有某个 Task"的变更必须经 Raft Leader 仲裁后落地。
//!
//! S00 阶段仅占位;S07 起接 Raft,S10 起实现 Scheduler,S11 起实现 StealArbiter。

pub struct RaftClient;
