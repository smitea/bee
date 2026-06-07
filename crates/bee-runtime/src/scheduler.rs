//! `RuntimeScheduler` — cooperative per-Node task priority (S22, ADR-0008).
//!
//! This is the **intra-Node** scheduler layer. The cross-Node
//! placement Scheduler (S10, in `bee-control`) decides which Node
//! runs a Task; the Runtime Scheduler (this module) decides which
//! **ready** Task on a single Node gets CPU share next.
//!
//! ## S22 scope
//! - [`TaskPriority`] enum: `High` / `Medium` / `Low`
//! - [`SchedulerPolicy`] enum: `Priority` (this scheduler) or
//!   `TokioDefault` (S10-style: no priority bias, plain
//!   `tokio::spawn`)
//! - [`RuntimeScheduler`] trait: `enqueue` + `next_ready` + `policy`
//! - [`PriorityRuntimeScheduler`] impl: per-priority `VecDeque`,
//!   `next_ready` returns the highest non-empty queue
//! - [`TokioDefaultScheduler`] impl: no-op, the caller falls back to
//!   plain `tokio::spawn`
//!
//! ## Cooperative, not preemptive
//! The scheduler **biases polling order**; a running Task is **not**
//! interrupted. Real preemption would need OS-level cgroup controls
//! (1.x per ADR-0008 §3). Cooperative means a high-priority Task
//! is more often the next to make progress, but a running low-priority
//! Task finishes its current handler call.
//!
//! ## S23 follow-up
//! MLFQ (Multi-Level Feedback Queue) with aging is layered on top
//! in S23. MLFQ keeps this trait's surface; the impl gains a
//! dynamic-priority demotion/promotion loop. SJF / SRTN / HRRN are
//! alternative `SchedulerPolicy` variants in S23.

use std::collections::VecDeque;
use std::sync::Mutex;

/// Stable identifier for a Task the scheduler is managing. Distinct
/// from the global `TaskId` in the ControlPlane — this is the
/// scheduler-local handle used to dequeue and dispatch.
pub type SchedulerTaskId = u64;

/// Priority levels the S22 scheduler understands. Higher priority
/// (lower numeric value) gets polled first. `Medium` is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum TaskPriority {
    High = 0,
    #[default]
    Medium = 1,
    Low = 2,
}

impl std::fmt::Display for TaskPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskPriority::High => f.write_str("high"),
            TaskPriority::Medium => f.write_str("medium"),
            TaskPriority::Low => f.write_str("low"),
        }
    }
}

/// Which scheduler a Bee Node uses at runtime.
///
/// S22 MVP: only `Priority` and `TokioDefault` are wired. S23 adds
/// `Mlfq`, `Sjf`, `Srtn`, `Hrrn` as alternative policies per ADR-0008.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SchedulerPolicy {
    /// S22 cooperative priority scheduler: high-priority ready
    /// Tasks are polled first.
    #[default]
    Priority,
    /// Fall back to plain `tokio::spawn` (S10 behavior). No
    /// priority bias; OS / tokio decide. The
    /// `TokioDefaultScheduler` is a no-op — the caller uses
    /// `tokio::spawn` directly.
    TokioDefault,
}

impl std::fmt::Display for SchedulerPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedulerPolicy::Priority => f.write_str("priority"),
            SchedulerPolicy::TokioDefault => f.write_str("tokio-default"),
        }
    }
}

/// The trait the runtime uses to decide which ready Task to poll
/// next. S22 defines the trait + two impls; S23 adds more.
pub trait RuntimeScheduler: Send + Sync {
    /// Enqueue a Task at the given priority. Called when a Task
    /// becomes ready (e.g. a Phase Handler finished a yield_now and
    /// is re-scheduled).
    fn enqueue(&self, task_id: SchedulerTaskId, priority: TaskPriority);

    /// Pop the highest-priority ready Task. Returns `None` if no
    /// Task is enqueued. For `TokioDefaultScheduler`, always
    /// returns `None` (the caller uses `tokio::spawn` directly).
    fn next_ready(&self) -> Option<(SchedulerTaskId, TaskPriority)>;

    /// Which policy this scheduler implements. Drives the
    /// `bee.runtime.scheduler_policy` config knob.
    fn policy(&self) -> SchedulerPolicy;

    /// Number of enqueued Tasks across all priorities (for
    /// `bee diagnostics`).
    fn ready_count(&self) -> usize;
}

/// S22 cooperative priority scheduler. Per-priority `VecDeque`s;
/// `next_ready` returns the head of the highest non-empty queue.
pub struct PriorityRuntimeScheduler {
    high: Mutex<VecDeque<SchedulerTaskId>>,
    medium: Mutex<VecDeque<SchedulerTaskId>>,
    low: Mutex<VecDeque<SchedulerTaskId>>,
}

impl Default for PriorityRuntimeScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityRuntimeScheduler {
    pub fn new() -> Self {
        Self {
            high: Mutex::new(VecDeque::new()),
            medium: Mutex::new(VecDeque::new()),
            low: Mutex::new(VecDeque::new()),
        }
    }
}

impl RuntimeScheduler for PriorityRuntimeScheduler {
    fn enqueue(&self, task_id: SchedulerTaskId, priority: TaskPriority) {
        let q = match priority {
            TaskPriority::High => &self.high,
            TaskPriority::Medium => &self.medium,
            TaskPriority::Low => &self.low,
        };
        q.lock().expect("scheduler mutex poisoned").push_back(task_id);
    }

    fn next_ready(&self) -> Option<(SchedulerTaskId, TaskPriority)> {
        if let Some(id) = self.high.lock().expect("poisoned").pop_front() {
            return Some((id, TaskPriority::High));
        }
        if let Some(id) = self.medium.lock().expect("poisoned").pop_front() {
            return Some((id, TaskPriority::Medium));
        }
        if let Some(id) = self.low.lock().expect("poisoned").pop_front() {
            return Some((id, TaskPriority::Low));
        }
        None
    }

    fn policy(&self) -> SchedulerPolicy {
        SchedulerPolicy::Priority
    }

    fn ready_count(&self) -> usize {
        let h = self.high.lock().expect("poisoned").len();
        let m = self.medium.lock().expect("poisoned").len();
        let l = self.low.lock().expect("poisoned").len();
        h + m + l
    }
}

/// S22 opt-out scheduler. `enqueue` and `next_ready` are no-ops;
/// the caller uses plain `tokio::spawn` (S10 behavior). Exists so
/// `bee.runtime.scheduler_policy = "tokio-default"` is a valid
/// config value that falls back to the S10 baseline.
pub struct TokioDefaultScheduler;

impl TokioDefaultScheduler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TokioDefaultScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeScheduler for TokioDefaultScheduler {
    fn enqueue(&self, _task_id: SchedulerTaskId, _priority: TaskPriority) {
        // No-op: the caller is expected to use tokio::spawn directly.
    }

    fn next_ready(&self) -> Option<(SchedulerTaskId, TaskPriority)> {
        None
    }

    fn policy(&self) -> SchedulerPolicy {
        SchedulerPolicy::TokioDefault
    }

    fn ready_count(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_default_is_medium() {
        assert_eq!(TaskPriority::default(), TaskPriority::Medium);
    }

    #[test]
    fn priority_ordering_is_high_lt_medium_lt_low() {
        // We use PartialOrd so the value that sorts first is
        // "highest" priority. The numeric enum discriminants give
        // us this for free.
        assert!(TaskPriority::High < TaskPriority::Medium);
        assert!(TaskPriority::Medium < TaskPriority::Low);
    }

    #[test]
    fn policy_default_is_priority() {
        assert_eq!(SchedulerPolicy::default(), SchedulerPolicy::Priority);
    }

    #[test]
    fn priority_scheduler_returns_high_first() {
        let sched = PriorityRuntimeScheduler::new();
        // Enqueue out-of-order: low, high, medium.
        sched.enqueue(1, TaskPriority::Low);
        sched.enqueue(2, TaskPriority::High);
        sched.enqueue(3, TaskPriority::Medium);

        let order: Vec<(u64, TaskPriority)> =
            std::iter::from_fn(|| sched.next_ready()).collect();
        assert_eq!(
            order,
            vec![
                (2, TaskPriority::High),
                (3, TaskPriority::Medium),
                (1, TaskPriority::Low),
            ]
        );
    }

    #[test]
    fn priority_scheduler_fifo_within_same_priority() {
        // Tasks at the same priority are FIFO: the first enqueued
        // gets polled first.
        let sched = PriorityRuntimeScheduler::new();
        for id in [10, 11, 12] {
            sched.enqueue(id, TaskPriority::Medium);
        }
        assert_eq!(sched.next_ready(), Some((10, TaskPriority::Medium)));
        assert_eq!(sched.next_ready(), Some((11, TaskPriority::Medium)));
        assert_eq!(sched.next_ready(), Some((12, TaskPriority::Medium)));
        assert_eq!(sched.next_ready(), None);
    }

    #[test]
    fn priority_scheduler_drains_high_before_medium() {
        // Multiple highs enqueued; all highs are polled before any
        // medium comes out.
        let sched = PriorityRuntimeScheduler::new();
        sched.enqueue(100, TaskPriority::Medium);
        sched.enqueue(1, TaskPriority::High);
        sched.enqueue(2, TaskPriority::High);
        sched.enqueue(101, TaskPriority::Medium);

        assert_eq!(sched.next_ready(), Some((1, TaskPriority::High)));
        assert_eq!(sched.next_ready(), Some((2, TaskPriority::High)));
        assert_eq!(sched.next_ready(), Some((100, TaskPriority::Medium)));
        assert_eq!(sched.next_ready(), Some((101, TaskPriority::Medium)));
    }

    #[test]
    fn priority_scheduler_three_tasks_with_different_priorities_s22_acceptance() {
        // S22 acceptance: 3 tasks with priorities [high, medium, low];
        // instrument which Task is polled in which order; assert
        // high comes first. With our deterministic implementation
        // it always comes first — the spec says "more often", and
        // "always" is the strictest "more often".
        let sched = PriorityRuntimeScheduler::new();
        for (id, p) in [
            (1u64, TaskPriority::High),
            (2, TaskPriority::Medium),
            (3, TaskPriority::Low),
        ] {
            sched.enqueue(id, p);
        }
        let first = sched.next_ready().expect("first task");
        assert_eq!(first, (1, TaskPriority::High));
    }

    #[test]
    fn priority_scheduler_ready_count_aggregates_across_queues() {
        let sched = PriorityRuntimeScheduler::new();
        assert_eq!(sched.ready_count(), 0);
        sched.enqueue(1, TaskPriority::High);
        sched.enqueue(2, TaskPriority::Medium);
        sched.enqueue(3, TaskPriority::Low);
        sched.enqueue(4, TaskPriority::Medium);
        assert_eq!(sched.ready_count(), 4);
        sched.next_ready();
        assert_eq!(sched.ready_count(), 3);
    }

    #[test]
    fn tokio_default_scheduler_is_no_op() {
        let sched = TokioDefaultScheduler::new();
        sched.enqueue(1, TaskPriority::High);
        assert_eq!(sched.next_ready(), None);
        assert_eq!(sched.ready_count(), 0);
        assert_eq!(sched.policy(), SchedulerPolicy::TokioDefault);
    }

    #[test]
    fn policy_trait_object_works() {
        // Both schedulers are usable through `&dyn RuntimeScheduler`
        // — the runtime doesn't need to know the concrete type.
        let sched: Box<dyn RuntimeScheduler> = Box::new(PriorityRuntimeScheduler::new());
        sched.enqueue(42, TaskPriority::High);
        assert_eq!(sched.next_ready(), Some((42, TaskPriority::High)));
        assert_eq!(sched.policy(), SchedulerPolicy::Priority);

        let sched: Box<dyn RuntimeScheduler> = Box::new(TokioDefaultScheduler::new());
        assert_eq!(sched.next_ready(), None);
        assert_eq!(sched.policy(), SchedulerPolicy::TokioDefault);
    }
}
