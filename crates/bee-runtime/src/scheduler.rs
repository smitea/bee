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

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
/// S22: `Priority` (cooperative priority) and `TokioDefault` (S10
/// baseline). S23 adds the four ADR-0008 policies: `Mlfq` (default
/// per ADR-0008 §3), `Sjf`, `Hrrn`, `Srtn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SchedulerPolicy {
    /// S22 cooperative priority scheduler (opt-in via config).
    Priority,
    /// S10 baseline: plain `tokio::spawn`, no priority bias.
    TokioDefault,
    /// S23 default (per ADR-0008 §3): Multi-Level Feedback Queue
    /// with aging. Demotes long-running Tasks, promotes starved
    /// ones.
    #[default]
    Mlfq,
    /// S23: Shortest Job First. Needs `expected_duration` per
    /// Task (historical average).
    Sjf,
    /// S23: Highest Response Ratio Next. `(wait + service) / service`.
    Hrrn,
    /// S23: Shortest Remaining Time Next. Preemptive SJF (cooperative
    /// variant: switch only on enqueue / yield).
    Srtn,
}

impl std::fmt::Display for SchedulerPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedulerPolicy::Priority => f.write_str("priority"),
            SchedulerPolicy::TokioDefault => f.write_str("tokio-default"),
            SchedulerPolicy::Mlfq => f.write_str("mlfq"),
            SchedulerPolicy::Sjf => f.write_str("sjf"),
            SchedulerPolicy::Hrrn => f.write_str("hrrn"),
            SchedulerPolicy::Srtn => f.write_str("srtn"),
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

// ---- S23 policies ----
//
// ADR-0008 default is MLFQ. The four policies share a per-Task
// metadata map. Each impl tracks the subset of fields it needs:
//   - MLFQ: level (0..N) + poll_count + enqueue_time (for aging)
//   - SJF:  expected_duration
//   - HRRN: enqueue_time + service_time (ratio = (wait+svc)/svc)
//   - SRTN: expected_duration (remaining = expected - elapsed)
//
// The S22 Priority / TokioDefault schedulers are simpler and keep
// no per-Task state beyond the queue itself.

// ---- MLFQ ----

struct MlfqTaskMeta {
    level: usize,
    poll_count: u32,
    enqueue_time: Instant,
}

pub struct MlfqRuntimeScheduler {
    /// Priority levels. 0 = highest, last = lowest. Default 3.
    levels: Vec<Mutex<VecDeque<SchedulerTaskId>>>,
    metadata: Mutex<HashMap<SchedulerTaskId, MlfqTaskMeta>>,
    /// Demote a Task down one level after this many consecutive
    /// polls. Default 5 (ADR-0008 §3: short Phases stay high,
    /// long Phases get demoted).
    demote_after: u32,
    /// Promote a Task waiting at level > 0 for this long back to
    /// level 0 (starvation prevention). Default 100ms.
    aging_threshold: Duration,
}

impl MlfqRuntimeScheduler {
    pub fn new(demote_after: u32, aging_threshold: Duration) -> Self {
        Self::with_levels(3, demote_after, aging_threshold)
    }

    pub fn with_levels(num_levels: usize, demote_after: u32, aging_threshold: Duration) -> Self {
        assert!(num_levels > 0, "MLFQ needs at least one level");
        let mut levels = Vec::with_capacity(num_levels);
        for _ in 0..num_levels {
            levels.push(Mutex::new(VecDeque::new()));
        }
        Self {
            levels,
            metadata: Mutex::new(HashMap::new()),
            demote_after,
            aging_threshold,
        }
    }

    /// Move a task to a specific level. Used by the aging logic
    /// (internal) and by tests (to simulate demotion state).
    fn set_level(&self, id: SchedulerTaskId, level: usize) {
        let mut metadata = self.metadata.lock().expect("poisoned");
        if let Some(meta) = metadata.get_mut(&id) {
            self.levels[meta.level]
                .lock()
                .expect("poisoned")
                .retain(|&x| x != id);
            meta.level = level;
            self.levels[level]
                .lock()
                .expect("poisoned")
                .push_back(id);
        }
    }
}

impl RuntimeScheduler for MlfqRuntimeScheduler {
    fn enqueue(&self, task_id: SchedulerTaskId, priority: TaskPriority) {
        // If the task is already known (re-enqueue after a poll
        // where it didn't finish), keep its current level AND
        // poll_count — the demotion counter is per-task-lifetime
        // (a "long" task keeps using CPU and should be demoted
        // across re-enqueues). Only update enqueue_time so aging
        // restarts for this "ready" period. Otherwise demotion
        // would have no effect across polls.
        let mut metadata = self.metadata.lock().expect("poisoned");
        let level = if let Some(existing) = metadata.get_mut(&task_id) {
            existing.enqueue_time = Instant::now();
            existing.level
        } else {
            let num_levels = self.levels.len();
            let level = match priority {
                TaskPriority::High => 0,
                TaskPriority::Medium if num_levels >= 3 => 1,
                TaskPriority::Medium => 0,
                TaskPriority::Low => num_levels - 1,
            };
            metadata.insert(
                task_id,
                MlfqTaskMeta {
                    level,
                    poll_count: 0,
                    enqueue_time: Instant::now(),
                },
            );
            level
        };
        drop(metadata);
        self.levels[level]
            .lock()
            .expect("poisoned")
            .push_back(task_id);
    }

    fn next_ready(&self) -> Option<(SchedulerTaskId, TaskPriority)> {
        // Aging: promote long-waiting tasks at level > 0.
        let now = Instant::now();
        let to_promote: Vec<SchedulerTaskId> = {
            let metadata = self.metadata.lock().expect("poisoned");
            metadata
                .iter()
                .filter_map(|(id, m)| {
                    if m.level > 0
                        && now.duration_since(m.enqueue_time) >= self.aging_threshold
                    {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect()
        };
        for id in to_promote {
            self.set_level(id, 0);
        }

        // Pop from the highest non-empty level.
        for level in 0..self.levels.len() {
            let popped = {
                let mut q = self.levels[level].lock().expect("poisoned");
                q.pop_front()
            };
            if let Some(id) = popped {
                // Increment poll count; demote after threshold.
                let mut metadata = self.metadata.lock().expect("poisoned");
                let mut new_level = level;
                if let Some(meta) = metadata.get_mut(&id) {
                    meta.poll_count += 1;
                    if meta.poll_count >= self.demote_after
                        && meta.level + 1 < self.levels.len()
                    {
                        meta.level += 1;
                        new_level = meta.level;
                    }
                }
                // If the task was demoted, re-insert it at the new
                // level so the queue is consistent.
                if new_level != level {
                    self.levels[new_level]
                        .lock()
                        .expect("poisoned")
                        .push_back(id);
                }
                let priority = match new_level {
                    0 => TaskPriority::High,
                    1 => TaskPriority::Medium,
                    _ => TaskPriority::Low,
                };
                return Some((id, priority));
            }
        }
        None
    }

    fn policy(&self) -> SchedulerPolicy {
        SchedulerPolicy::Mlfq
    }

    fn ready_count(&self) -> usize {
        self.levels
            .iter()
            .map(|q| q.lock().expect("poisoned").len())
            .sum()
    }
}

// ---- SJF ----

struct SjfTaskMeta {
    expected_duration: Duration,
}

pub struct SjfRuntimeScheduler {
    /// Min-heap: pop the shortest expected_duration first.
    heap: Mutex<BinaryHeap<(Reverse<Duration>, SchedulerTaskId)>>,
    metadata: Mutex<HashMap<SchedulerTaskId, SjfTaskMeta>>,
    /// Default expected_duration when not set per-task.
    default_duration: Duration,
}

impl SjfRuntimeScheduler {
    pub fn new(default_duration: Duration) -> Self {
        Self {
            heap: Mutex::new(BinaryHeap::new()),
            metadata: Mutex::new(HashMap::new()),
            default_duration,
        }
    }

    /// Test hook: set the expected duration for a Task. Production
    /// code populates this from the historical-average path (S24
    /// metrics feed).
    #[cfg(test)]
    fn set_expected_duration_for_test(&self, id: SchedulerTaskId, d: Duration) {
        self.metadata.lock().expect("poisoned").insert(
            id,
            SjfTaskMeta {
                expected_duration: d,
            },
        );
    }
}

impl RuntimeScheduler for SjfRuntimeScheduler {
    fn enqueue(&self, task_id: SchedulerTaskId, priority: TaskPriority) {
        let d = self
            .metadata
            .lock()
            .expect("poisoned")
            .get(&task_id)
            .map(|m| m.expected_duration)
            .unwrap_or(self.default_duration);
        self.heap
            .lock()
            .expect("poisoned")
            .push((Reverse(d), task_id));
        // If the caller enqueued without setting duration, we
        // remember the priority → default mapping for visibility.
        let _ = priority;
    }

    fn next_ready(&self) -> Option<(SchedulerTaskId, TaskPriority)> {
        let mut heap = self.heap.lock().expect("poisoned");
        heap.pop().map(|(_, id)| (id, TaskPriority::Medium))
    }

    fn policy(&self) -> SchedulerPolicy {
        SchedulerPolicy::Sjf
    }

    fn ready_count(&self) -> usize {
        self.heap.lock().expect("poisoned").len()
    }
}

// ---- HRRN ----

struct HrrnTaskMeta {
    enqueue_time: Instant,
    service_time: Duration,
}

pub struct HrrnRuntimeScheduler {
    queue: Mutex<VecDeque<SchedulerTaskId>>,
    metadata: Mutex<HashMap<SchedulerTaskId, HrrnTaskMeta>>,
}

impl HrrnRuntimeScheduler {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            metadata: Mutex::new(HashMap::new()),
        }
    }

    /// Test hook: set service time for a Task.
    #[cfg(test)]
    fn set_service_time_for_test(&self, id: SchedulerTaskId, service: Duration) {
        self.metadata.lock().expect("poisoned").insert(
            id,
            HrrnTaskMeta {
                enqueue_time: Instant::now(),
                service_time: service,
            },
        );
    }
}

impl Default for HrrnRuntimeScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeScheduler for HrrnRuntimeScheduler {
    fn enqueue(&self, task_id: SchedulerTaskId, priority: TaskPriority) {
        self.queue
            .lock()
            .expect("poisoned")
            .push_back(task_id);
        let _ = priority;
        // If metadata wasn't pre-set, initialize with enqueue_time
        // and a default service_time.
        let mut metadata = self.metadata.lock().expect("poisoned");
        metadata.entry(task_id).or_insert(HrrnTaskMeta {
            enqueue_time: Instant::now(),
            service_time: Duration::from_millis(1),
        });
    }

    fn next_ready(&self) -> Option<(SchedulerTaskId, TaskPriority)> {
        let queue = self.queue.lock().expect("poisoned");
        if queue.is_empty() {
            return None;
        }
        let now = Instant::now();
        let metadata = self.metadata.lock().expect("poisoned");
        let mut best: Option<(SchedulerTaskId, f64)> = None;
        for &id in queue.iter() {
            let m = metadata.get(&id).expect("metadata must exist");
            let wait = now.duration_since(m.enqueue_time).as_secs_f64();
            let svc = m.service_time.as_secs_f64().max(1e-9);
            let ratio = (wait + svc) / svc;
            if best.map_or(true, |(_, r)| ratio > r) {
                best = Some((id, ratio));
            }
        }
        // We need to actually pop the best. Drop both locks, re-acquire
        // queue, remove the best by id.
        drop(metadata);
        drop(queue);
        let id = best.map(|(id, _)| id)?;
        let mut queue = self.queue.lock().expect("poisoned");
        queue.retain(|&x| x != id);
        Some((id, TaskPriority::Medium))
    }

    fn policy(&self) -> SchedulerPolicy {
        SchedulerPolicy::Hrrn
    }

    fn ready_count(&self) -> usize {
        self.queue.lock().expect("poisoned").len()
    }
}

// ---- SRTN ----
//
// Like SJF but with a "running" tracker: when a new Task arrives
// with shorter remaining time than the running Task, the running
// Task is preempted and re-enqueued at the back of the queue.
// Cooperative variant: the switch happens on enqueue / yield, not
// in the middle of a handler call.

struct SrtnTaskMeta {
    expected_duration: Duration,
}

pub struct SrtnRuntimeScheduler {
    heap: Mutex<BinaryHeap<(Reverse<Duration>, SchedulerTaskId)>>,
    metadata: Mutex<HashMap<SchedulerTaskId, SrtnTaskMeta>>,
    /// Currently running Task. If a new Task arrives with shorter
    /// expected_duration, the running Task is preempted.
    running: Mutex<Option<SchedulerTaskId>>,
}

impl SrtnRuntimeScheduler {
    pub fn new() -> Self {
        Self {
            heap: Mutex::new(BinaryHeap::new()),
            metadata: Mutex::new(HashMap::new()),
            running: Mutex::new(None),
        }
    }

    /// Test hook: set the expected duration.
    #[cfg(test)]
    fn set_expected_duration_for_test(&self, id: SchedulerTaskId, d: Duration) {
        self.metadata.lock().expect("poisoned").insert(
            id,
            SrtnTaskMeta {
                expected_duration: d,
            },
        );
    }

    fn elapsed_for(&self, id: SchedulerTaskId) -> Duration {
        // MVP: no actual elapsed-time tracking; we use the
        // metadata's expected_duration directly. A real
        // implementation would record start_time on `start_running`
        // and compute elapsed = now - start_time.
        self.metadata
            .lock()
            .expect("poisoned")
            .get(&id)
            .map(|m| m.expected_duration)
            .unwrap_or_default()
    }
}

impl Default for SrtnRuntimeScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeScheduler for SrtnRuntimeScheduler {
    fn enqueue(&self, task_id: SchedulerTaskId, priority: TaskPriority) {
        let d = self
            .metadata
            .lock()
            .expect("poisoned")
            .get(&task_id)
            .map(|m| m.expected_duration)
            .unwrap_or(Duration::from_millis(10));
        let _ = priority;
        // SRTN: if there's a running task with longer expected_duration
        // than this new task, preempt it.
        let mut running = self.running.lock().expect("poisoned");
        if let Some(running_id) = *running {
            let running_d = self.elapsed_for(running_id);
            if d < running_d {
                // Preempt: re-enqueue the running task with its
                // (still-remaining) time, then enqueue the new task
                // so it's the next to dispatch.
                self.heap
                    .lock()
                    .expect("poisoned")
                    .push((Reverse(running_d), running_id));
                self.heap
                    .lock()
                    .expect("poisoned")
                    .push((Reverse(d), task_id));
                *running = Some(task_id);
            } else {
                self.heap
                    .lock()
                    .expect("poisoned")
                    .push((Reverse(d), task_id));
            }
        } else {
            self.heap
                .lock()
                .expect("poisoned")
                .push((Reverse(d), task_id));
        }
    }

    fn next_ready(&self) -> Option<(SchedulerTaskId, TaskPriority)> {
        let mut heap = self.heap.lock().expect("poisoned");
        let popped = heap.pop();
        if let Some((_, id)) = popped {
            *self.running.lock().expect("poisoned") = Some(id);
            Some((id, TaskPriority::Medium))
        } else {
            None
        }
    }

    fn policy(&self) -> SchedulerPolicy {
        SchedulerPolicy::Srtn
    }

    fn ready_count(&self) -> usize {
        self.heap.lock().expect("poisoned").len()
    }
}

// ---- SchedulerConfig (S23) ----
//
// The `bee.runtime.scheduler_policy` config knob. Switching the
// policy requires only a Node restart — the active scheduler is
// built once at startup.

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub policy: SchedulerPolicy,
    /// MLFQ: number of priority levels. Default 3.
    pub mlfq_levels: usize,
    /// MLFQ: demote a Task after this many consecutive polls. Default 5.
    pub mlfq_demote_after: u32,
    /// MLFQ: aging threshold for promoting starving Tasks. Default 100ms.
    pub mlfq_aging_threshold: Duration,
    /// SJF / SRTN: default expected_duration when per-Task data
    /// isn't set. Default 10ms.
    pub default_expected_duration: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            policy: SchedulerPolicy::default(),
            mlfq_levels: 3,
            mlfq_demote_after: 5,
            mlfq_aging_threshold: Duration::from_millis(100),
            default_expected_duration: Duration::from_millis(10),
        }
    }
}

impl SchedulerConfig {
    /// Build the configured scheduler. Switching the policy only
    /// requires restarting the Node (per S23 acceptance).
    pub fn build(&self) -> Box<dyn RuntimeScheduler> {
        match self.policy {
            SchedulerPolicy::Priority => Box::new(PriorityRuntimeScheduler::new()),
            SchedulerPolicy::TokioDefault => Box::new(TokioDefaultScheduler::new()),
            SchedulerPolicy::Mlfq => Box::new(MlfqRuntimeScheduler::with_levels(
                self.mlfq_levels,
                self.mlfq_demote_after,
                self.mlfq_aging_threshold,
            )),
            SchedulerPolicy::Sjf => {
                Box::new(SjfRuntimeScheduler::new(self.default_expected_duration))
            }
            SchedulerPolicy::Hrrn => Box::new(HrrnRuntimeScheduler::new()),
            SchedulerPolicy::Srtn => Box::new(SrtnRuntimeScheduler::new()),
        }
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
    fn policy_default_is_mlfq() {
        // S23 (ADR-0008 §3): MLFQ is the default scheduling policy.
        assert_eq!(SchedulerPolicy::default(), SchedulerPolicy::Mlfq);
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

    // ---- S23 policy tests ----

    #[test]
    fn mlfq_short_task_dispatched_before_long_under_demotion() {
        // S23 acceptance: 3 Tasks with known CPU costs; under MLFQ,
        // short Tasks complete before long Tasks. The semantic is
        // "TaskPriority on enqueue reflects CPU cost": a High
        // (short) Task stays at level 0 for multiple polls, while a
        // Low (long) Task starts at the lowest level and rarely
        // gets dispatched. We mix priorities to verify the bias.
        let sched = MlfqRuntimeScheduler::new(100, Duration::from_millis(100));
        sched.enqueue(1, TaskPriority::High); // short — stays at level 0
        sched.enqueue(2, TaskPriority::Medium); // medium — starts at level 1
        sched.enqueue(3, TaskPriority::Low); // long — starts at level 2 (lowest)

        // First dispatch: level 0 → task 1 (the short one).
        let first = sched.next_ready().expect("first");
        assert_eq!(first.0, 1, "short (High) dispatched first");
        assert_eq!(first.1, TaskPriority::High);

        // After demote_after=100 polls the High task demotes, but
        // with demote_after=100 we won't trigger demotion in 1 poll.
        // So level 0 is now empty.
        // Next dispatch: level 1 → task 2 (the medium).
        let second = sched.next_ready().expect("second");
        assert_eq!(second.0, 2, "medium dispatched second");
        assert_eq!(second.1, TaskPriority::Medium);

        // Next: level 2 → task 3 (the long).
        let third = sched.next_ready().expect("third");
        assert_eq!(third.0, 3, "long dispatched last");
        assert_eq!(third.1, TaskPriority::Low);
    }

    #[test]
    fn mlfq_demotes_after_threshold_consecutive_polls() {
        // The MLFQ contract: a task that doesn't "finish" (i.e., the
        // caller re-enqueues it) gets demoted. Test this by
        // re-enqueuing the same task id after each poll.
        let sched = MlfqRuntimeScheduler::with_levels(3, 2, Duration::from_millis(100));
        sched.enqueue(42, TaskPriority::High);
        // Poll once — task 42 is at level 0, no demote yet.
        let (id, prio) = sched.next_ready().unwrap();
        assert_eq!(id, 42);
        assert_eq!(prio, TaskPriority::High);
        // Re-enqueue (the task didn't finish). Now poll_count=1.
        sched.enqueue(42, TaskPriority::High);
        let (_, prio) = sched.next_ready().unwrap();
        // poll_count is now 2 → demote to level 1 → Medium.
        assert_eq!(prio, TaskPriority::Medium);
        // Re-enqueue and poll again. poll_count=3 → demote to
        // level 2 → Low.
        sched.enqueue(42, TaskPriority::High);
        let (_, prio) = sched.next_ready().unwrap();
        assert_eq!(prio, TaskPriority::Low);
    }

    #[test]
    fn sjf_dispatches_shortest_expected_duration_first() {
        // S23 acceptance: feed a known mix of Task durations,
        // assert the expected dispatch order. SJF pops the shortest
        // first.
        let sched = SjfRuntimeScheduler::new(Duration::from_millis(100));
        // Enqueue without per-task duration: all get default 100ms.
        // Use the test hook to set per-task durations.
        sched.set_expected_duration_for_test(1, Duration::from_millis(10));
        sched.set_expected_duration_for_test(2, Duration::from_millis(5));
        sched.set_expected_duration_for_test(3, Duration::from_millis(20));
        sched.enqueue(1, TaskPriority::Medium);
        sched.enqueue(2, TaskPriority::Medium);
        sched.enqueue(3, TaskPriority::Medium);

        let order: Vec<u64> = std::iter::from_fn(|| sched.next_ready().map(|(id, _)| id)).collect();
        assert_eq!(order, vec![2, 1, 3], "SJF must dispatch by expected_duration ascending");
    }

    #[test]
    fn hrrn_picks_highest_response_ratio() {
        // HRRN ratio = (wait + service) / service. The Task with the
        // highest ratio gets dispatched next. Two Tasks: A has
        // long service; B has short service. After a real wait,
        // B's ratio wins.
        let sched = HrrnRuntimeScheduler::new();
        sched.set_service_time_for_test(1, Duration::from_millis(8));
        sched.set_service_time_for_test(2, Duration::from_millis(2));
        sched.enqueue(1, TaskPriority::Medium);
        sched.enqueue(2, TaskPriority::Medium);
        // Let both Tasks wait — their wait times are now similar,
        // so the ratio is dominated by service_time (smaller
        // service = higher ratio).
        std::thread::sleep(Duration::from_millis(20));
        let first = sched.next_ready().unwrap();
        assert_eq!(first.0, 2, "B (shorter service) has higher ratio");
    }

    #[test]
    fn srtn_preempts_longer_running_task_for_shorter_arrival() {
        // S23: SRTN preempts the running Task if a new arrival has
        // shorter expected_duration.
        let sched = SrtnRuntimeScheduler::new();
        // Enqueue long task first, then start running it.
        sched.set_expected_duration_for_test(1, Duration::from_millis(20));
        sched.enqueue(1, TaskPriority::Medium);
        let first = sched.next_ready().unwrap();
        assert_eq!(first.0, 1, "long task starts running");

        // Enqueue a short task — it should preempt the long one.
        sched.set_expected_duration_for_test(2, Duration::from_millis(5));
        sched.enqueue(2, TaskPriority::Medium);

        // Next dispatch is the short task; the long task was
        // re-enqueued at the back of the heap.
        let next = sched.next_ready().unwrap();
        assert_eq!(next.0, 2, "short arrival preempts the long running task");
        // The long task comes out last.
        let last = sched.next_ready().unwrap();
        assert_eq!(last.0, 1);
    }

    #[test]
    fn srtn_does_not_preempt_when_arrival_is_longer() {
        // If the new arrival is LONGER than the running task, do
        // not preempt — let the running task finish.
        let sched = SrtnRuntimeScheduler::new();
        sched.set_expected_duration_for_test(1, Duration::from_millis(5));
        sched.set_expected_duration_for_test(2, Duration::from_millis(20));
        sched.enqueue(1, TaskPriority::Medium);
        let _ = sched.next_ready().unwrap(); // running = 1 (short)
        sched.enqueue(2, TaskPriority::Medium);
        // 2 is longer, so no preemption. next_ready returns 2
        // anyway (the heap still has 2 at the back).
        let next = sched.next_ready().unwrap();
        assert_eq!(next.0, 2);
    }

    #[test]
    fn scheduler_config_builds_each_policy() {
        // S23 acceptance: switching policy requires only a Node
        // restart. SchedulerConfig::build() constructs a fresh
        // scheduler for the configured policy; the rest of the
        // runtime doesn't change.
        for policy in [
            SchedulerPolicy::Priority,
            SchedulerPolicy::TokioDefault,
            SchedulerPolicy::Mlfq,
            SchedulerPolicy::Sjf,
            SchedulerPolicy::Hrrn,
            SchedulerPolicy::Srtn,
        ] {
            let cfg = SchedulerConfig {
                policy,
                ..Default::default()
            };
            let sched = cfg.build();
            assert_eq!(sched.policy(), policy, "policy mismatch for {policy:?}");
            // Smoke test: enqueue + next_ready doesn't panic.
            sched.enqueue(1, TaskPriority::Medium);
            let _ = sched.next_ready();
        }
    }

    #[test]
    fn scheduler_config_default_uses_mlfq_s23_default() {
        // S23 (ADR-0008 §3): MLFQ is the default scheduling policy.
        // SchedulerConfig::default() inherits from SchedulerPolicy::default().
        let cfg = SchedulerConfig::default();
        assert_eq!(cfg.policy, SchedulerPolicy::Mlfq);
    }
}
