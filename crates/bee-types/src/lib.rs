//! `bee-types` — 跨 crate 共享的核心类型。
//!
//! 这个 crate 的存在是为了打破 `bee-control` ↔ `bee-runtime` 之间的循环依赖:
//! - `bee-control` 依赖 `bee-runtime` (Handler / Dag / RuntimeError)
//! - `bee-runtime` 需要 `JobLifecycleState` 来给 `StreamSubscriber` 状态机使用
//!   (S17 §5)
//!
//! 把生命周期枚举抽到零依赖的 `bee-types` 后,`bee-runtime` 可以依赖 `bee-types`
//! 而不必拉整个 `bee-control` 进来。
//!
//! ## 当前包含
//!
//! - [`JobLifecycleState`] — Pipeline Job 的高层生命周期 (Pending /
//!   Scheduled / WaitingForUpstream / Running / Completed / Failed),
//!   S18 引入,被 S17 §3 propagate_producer_death、S17 §5 StreamSubscriber
//!   等多个模块共同消费。

/// S18: high-level lifecycle of a Pipeline Job. `WaitingForUpstream`
/// means the Job is registered but at least one declared dependency
/// (`Job B depends on Job A's output`) has not yet reached `Running`.
/// The deployer / orchestrator drives transitions: Pending → Scheduled
/// → Running (or WaitingForUpstream → Running once deps are met).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize,
)]
pub enum JobLifecycleState {
    #[default]
    Pending,
    Scheduled,
    /// S18: dependencies declared but not all upstreams are `Running`.
    WaitingForUpstream,
    Running,
    Completed,
    Failed,
}
