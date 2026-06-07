//! Scheduler — Task-to-Node placement by resource declaration.
//!
//! Per architecture §5.2 / ADR-0008, the Scheduler is the
//! `Scheduler` block (cluster-scoped, control-plane). It receives a
//! set of Task requirements (cpu_millicores, mem_mb) and a set of
//! Node capacities, and returns a placement plan.
//!
//! The default `FirstFitDecreasingScheduler` is a classic FFD bin-packing
//! heuristic: sort tasks by `max(cpu, mem)` descending, then for each
//! task place it on the first node with enough remaining capacity. This
//! is provably (4/3)-approximate for makespan-minimization and is good
//! enough for the MVP. S25 will replace it with the adaptive MLFQ-aware
//! strategy that consults runtime metrics.
//!
//! The trait is `Send + Sync` and object-safe so the Deployer can hold
//! a `Box<dyn Scheduler>` and tests can swap in custom strategies.

pub struct TaskRequirement {
    pub task_id: u32,
    pub cpu_millicores: u32,
    pub mem_mb: u32,
}

pub struct NodeCapacity {
    pub node_id: u32,
    pub cpu_millicores_total: u32,
    pub mem_mb_total: u32,
}

pub struct TaskPlacement {
    pub task_id: u32,
    pub node_id: u32,
}

pub trait Scheduler: Send + Sync {
    /// Returns one `Option<TaskPlacement>` per input task (same order).
    /// `None` means the task could not be placed (insufficient capacity
    /// on every node).
    fn place(
        &self,
        tasks: &[TaskRequirement],
        nodes: &[NodeCapacity],
    ) -> Vec<Option<TaskPlacement>>;
}

pub struct FirstFitDecreasingScheduler;

impl FirstFitDecreasingScheduler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FirstFitDecreasingScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler for FirstFitDecreasingScheduler {
    fn place(
        &self,
        tasks: &[TaskRequirement],
        nodes: &[NodeCapacity],
    ) -> Vec<Option<TaskPlacement>> {
        if nodes.is_empty() {
            return (0..tasks.len()).map(|_| None).collect();
        }

        let mut indexed: Vec<usize> = (0..tasks.len()).collect();
        indexed.sort_by_key(|&i| {
            let t = &tasks[i];
            std::cmp::max(t.cpu_millicores, t.mem_mb)
        });
        indexed.reverse();

        let mut remaining: Vec<(u32, u32)> = nodes
            .iter()
            .map(|n| (n.cpu_millicores_total, n.mem_mb_total))
            .collect();

        let mut result: Vec<Option<TaskPlacement>> = (0..tasks.len()).map(|_| None).collect();

        for &i in &indexed {
            let task = &tasks[i];
            for (node_idx, node) in nodes.iter().enumerate() {
                let (cpu_left, mem_left) = remaining[node_idx];
                if task.cpu_millicores <= cpu_left && task.mem_mb <= mem_left {
                    result[i] = Some(TaskPlacement {
                        task_id: task.task_id,
                        node_id: node.node_id,
                    });
                    remaining[node_idx] = (
                        cpu_left - task.cpu_millicores,
                        mem_left - task.mem_mb,
                    );
                    break;
                }
            }
        }

        result
    }
}

/// A trivial pluggable scheduler that pins all tasks to node 0.
/// Used by the pluggability test to confirm the Scheduler trait
/// is the swap point (not hardcoded into the Deployer).
pub struct PinToFirstNode;

impl Scheduler for PinToFirstNode {
    fn place(
        &self,
        tasks: &[TaskRequirement],
        nodes: &[NodeCapacity],
    ) -> Vec<Option<TaskPlacement>> {
        let Some(first) = nodes.first() else {
            return (0..tasks.len()).map(|_| None).collect();
        };
        let pinned = first.node_id;
        tasks
            .iter()
            .map(|t| Some(TaskPlacement { task_id: t.task_id, node_id: pinned }))
            .collect()
    }
}

