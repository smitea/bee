//! Control Plane state machine — Job/Task metadata in Raft.
//!
//! S08 (this slice) adds a second logical state machine that coexists with
//! the KV state machine on the same Raft group. Both SMs receive the same
//! committed log entries; dispatch is by `Op` variant. The apply path
//! (`Node::apply_committed`) locks both SMs once and dispatches each entry
//! to the appropriate one.
//!
//! Per architecture §4.2 + ADR-0001, the ControlPlane is the source of
//! truth for Job/Task ownership. The KV SM is for opaque per-Task state
//! (handler-managed, see ADR-0004). Both replicate through the same Raft
//! group but are independent: a Put on KV never affects a Job, and a
//! RegisterJob never affects the KV store.

use std::collections::HashMap;

use crate::kv::{JobLifecycleState, Op, TaskStatus, TxnError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    pub job_id: u32,
    pub dag_hash: String,
    pub owner_node: u32,
    /// S18: lifecycle state. Defaults to `Pending` on `RegisterJob`.
    pub lifecycle: JobLifecycleState,
    /// S18: declared upstream dependencies. A Job with a non-empty
    /// `dependencies` list is `WaitingForUpstream` until each listed
    /// upstream is `Running`. The list is order-insensitive (the
    /// deployer's orchestrator can satisfy deps in any order).
    pub dependencies: Vec<DependencyRecord>,
}

/// S18: one upstream dependency declared by a downstream Job — for
/// example, \"Job B reads from Job A's `output` stream\". Resolved
/// at deploy time: same Node → in-process edge; different Node → BRP
/// data channel subscription. S18 stops at the metadata layer; the
/// actual data-channel resolution is the S25 cross-Node rebalance
/// machinery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyRecord {
    pub upstream_job: u32,
    pub stream: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    pub task_id: u32,
    pub job_id: u32,
    pub phase_id: u32,
    pub owner_node: u32,
    pub status: TaskStatus,
    /// S25: wall-clock millis when the Task was assigned to its
    /// current owner. The Rebalancer uses this to gate
    /// rebalance on the `min_task_age_secs` threshold.
    pub started_at_ms: u64,
}

#[derive(Debug, Default, Clone)]
pub struct ControlPlaneStateMachine {
    jobs: HashMap<u32, JobRecord>,
    tasks: HashMap<u32, TaskRecord>,
    /// Per-node last heartbeat timestamp (ms since unix epoch). Updated by
    /// the `Op::Heartbeat` apply path; the S11 orchestrator uses this
    /// to detect missing nodes and mark their tasks as `Orphaned`.
    last_heartbeat: HashMap<u32, u64>,
    /// S17 Producer/Subscriber registry: `DatasourceSignature -> JobId`.
    /// The first writer wins — subsequent deploys of the same signature
    /// become Subscribers pointing at the existing Producer.
    datasource_producers: HashMap<String, u32>,
}

impl ControlPlaneStateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_op(&mut self, op: &Op) -> Result<(), TxnError> {
        match op {
            Op::RegisterJob { job_id, dag_hash, owner_node } => {
                // Preserve existing lifecycle + dependencies on a
                // re-Register of the same job_id (the Deployer may
                // re-register after a dep change).
                let existing = self.jobs.get(job_id).cloned();
                self.jobs.insert(
                    *job_id,
                    JobRecord {
                        job_id: *job_id,
                        dag_hash: dag_hash.clone(),
                        owner_node: *owner_node,
                        lifecycle: existing
                            .as_ref()
                            .map(|j| j.lifecycle)
                            .unwrap_or_default(),
                        dependencies: existing
                            .map(|j| j.dependencies)
                            .unwrap_or_default(),
                    },
                );
                Ok(())
            }
            Op::RegisterTask {
                task_id,
                job_id,
                phase_id,
                owner_node,
                status,
                started_at_ms,
            } => {
                self.tasks.insert(
                    *task_id,
                    TaskRecord {
                        task_id: *task_id,
                        job_id: *job_id,
                        phase_id: *phase_id,
                        owner_node: *owner_node,
                        status: *status,
                        started_at_ms: *started_at_ms,
                    },
                );
                Ok(())
            }
            Op::UpdateTaskStatus { task_id, new_status } => {
                if let Some(task) = self.tasks.get_mut(task_id) {
                    task.status = *new_status;
                    Ok(())
                } else {
                    Err(TxnError::Conflict {
                        key: format!("task_{task_id}_not_found"),
                        expected: None,
                        actual: None,
                    })
                }
            }
            Op::Heartbeat {
                node_id,
                timestamp_ms,
            } => {
                self.last_heartbeat.insert(*node_id, *timestamp_ms);
                Ok(())
            }
            Op::StealTask {
                thief_node,
                task_id,
            } => {
                // Atomic check-and-set: only the first StealTask for an
                // Orphaned task succeeds. Subsequent ones find the task
                // already in `Migrating` and return Conflict (no-op
                // at the Raft log level; the thief reads the CP to learn
                // the outcome).
                let Some(task) = self.tasks.get_mut(task_id) else {
                    return Err(TxnError::Conflict {
                        key: format!("task_{task_id}_not_found"),
                        expected: None,
                        actual: None,
                    });
                };
                if task.status != TaskStatus::Orphaned {
                    return Err(TxnError::Conflict {
                        key: format!("task_{task_id}_not_orphaned"),
                        expected: Some(format!("{:?}", TaskStatus::Orphaned).into_bytes()),
                        actual: Some(format!("{:?}", task.status).into_bytes()),
                    });
                }
                task.status = TaskStatus::Migrating;
                task.owner_node = *thief_node;
                Ok(())
            }
            Op::RegisterDatasourceProducer { signature, job_id } => {
                // Idempotent: first writer wins. Subsequent deploys
                // with the same signature become Subscribers pointing
                // at the existing producer (ADR-0003).
                if let std::collections::hash_map::Entry::Vacant(e) =
                    self.datasource_producers.entry(signature.clone())
                {
                    e.insert(*job_id);
                }
                Ok(())
            }
            Op::RegisterDependency {
                downstream_job,
                upstream_job,
                stream,
            } => {
                let job = self.jobs.get_mut(downstream_job).ok_or_else(|| {
                    TxnError::Conflict {
                        key: format!("job_{downstream_job}_not_found"),
                        expected: None,
                        actual: None,
                    }
                })?;
                let dep = DependencyRecord {
                    upstream_job: *upstream_job,
                    stream: stream.clone(),
                };
                let was_new = !job.dependencies.contains(&dep);
                if was_new {
                    job.dependencies.push(dep);
                }
                // Any new dependency must be re-evaluated: if the
                // upstream is not Running, the Job must enter
                // WaitingForUpstream. The orchestrator's tick will
                // promote it to Running once the new dep is satisfied.
                // (Idempotent re-RegisterDependency on a Job already
                // WaitingForUpstream is a no-op on lifecycle.)
                if was_new
                    && matches!(
                        job.lifecycle,
                        JobLifecycleState::Running
                            | JobLifecycleState::Pending
                            | JobLifecycleState::Scheduled
                    )
                {
                    job.lifecycle = JobLifecycleState::WaitingForUpstream;
                }
                Ok(())
            }
            Op::UpdateJobLifecycle { job_id, state } => {
                let job = self.jobs.get_mut(job_id).ok_or_else(|| {
                    TxnError::Conflict {
                        key: format!("job_{job_id}_not_found"),
                        expected: None,
                        actual: None,
                    }
                })?;
                job.lifecycle = *state;
                Ok(())
            }
            Op::Put { .. }
            | Op::Del { .. }
            | Op::Cas { .. }
            | Op::Txn { .. } => Err(TxnError::WrongSm),
        }
    }

    pub fn list_jobs(&self) -> Vec<JobRecord> {
        let mut v: Vec<JobRecord> = self.jobs.values().cloned().collect();
        v.sort_by_key(|j| j.job_id);
        v
    }

    /// S17: look up the JobId of the Producer for a given
    /// `DatasourceSignature`. Returns `None` if no Producer has been
    /// registered yet — the deployer treats that case as "this deploy
    /// is the Producer".
    pub fn lookup_datasource_producer(&self, signature: &str) -> Option<u32> {
        self.datasource_producers.get(signature).copied()
    }

    /// S17: number of registered Producers. Used by tests and the
    /// (future) `bee jobs list` admin surface to show the
    /// Producer/Subscriber breakdown.
    pub fn datasource_producer_count(&self) -> usize {
        self.datasource_producers.len()
    }

    pub fn list_tasks(&self) -> Vec<TaskRecord> {
        let mut v: Vec<TaskRecord> = self.tasks.values().cloned().collect();
        v.sort_by_key(|t| t.task_id);
        v
    }

    pub fn get_job(&self, job_id: u32) -> Option<&JobRecord> {
        self.jobs.get(&job_id)
    }

    /// S18: returns true if every declared upstream dependency of
    /// `job_id` exists in the CP and is currently `Running`. A Job
    /// with no dependencies is trivially satisfied.
    pub fn job_dependencies_satisfied(&self, job_id: u32) -> bool {
        let Some(job) = self.jobs.get(&job_id) else {
            return false;
        };
        job.dependencies.iter().all(|d| {
            self.jobs
                .get(&d.upstream_job)
                .is_some_and(|u| u.lifecycle == JobLifecycleState::Running)
        })
    }

    /// S18: returns the lifecycle state that the orchestrator should
    /// drive `job_id` toward. Pure function — does not mutate. Used
    /// by the orchestrator to decide whether to submit an
    /// `UpdateJobLifecycle` op.
    pub fn evaluate_job_state(&self, job_id: u32) -> JobLifecycleState {
        let Some(job) = self.jobs.get(&job_id) else {
            return JobLifecycleState::Pending;
        };
        // Already terminal → keep it.
        if matches!(
            job.lifecycle,
            JobLifecycleState::Completed | JobLifecycleState::Failed
        ) {
            return job.lifecycle;
        }
        // No deps → if Pending/Scheduled/Waiting, go Running.
        if job.dependencies.is_empty() {
            return JobLifecycleState::Running;
        }
        // Has deps → satisfied → Running, otherwise Waiting.
        if self.job_dependencies_satisfied(job_id) {
            JobLifecycleState::Running
        } else {
            JobLifecycleState::WaitingForUpstream
        }
    }

    /// S18: list of downstream Jobs that declared a dependency on
    /// `upstream_job`. Used by the orchestrator when the upstream
    /// transitions to Running (so it can re-evaluate the
    /// downstream's deps and possibly promote it).
    pub fn downstream_jobs_of(&self, upstream_job: u32) -> Vec<u32> {
        self.jobs
            .values()
            .filter(|j| {
                j.dependencies
                    .iter()
                    .any(|d| d.upstream_job == upstream_job)
            })
            .map(|j| j.job_id)
            .collect()
    }

    pub fn get_task(&self, task_id: u32) -> Option<&TaskRecord> {
        self.tasks.get(&task_id)
    }

    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// `node_id` is stale if its last heartbeat was more than
    /// `threshold_ms` ago (or if it has never reported in).
    pub fn stale_nodes(&self, now_ms: u64, threshold_ms: u64) -> Vec<u32> {
        self.last_heartbeat
            .iter()
            .filter_map(|(&node_id, &last_ms)| {
                if now_ms.saturating_sub(last_ms) > threshold_ms {
                    Some(node_id)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn last_heartbeat(&self, node_id: u32) -> Option<u64> {
        self.last_heartbeat.get(&node_id).copied()
    }

    /// Tasks owned by `node_id` (used by orphan detection to find what
    /// to mark as `Orphaned`).
    pub fn tasks_owned_by(&self, node_id: u32) -> Vec<TaskRecord> {
        self.tasks
            .values()
            .filter(|t| t.owner_node == node_id)
            .cloned()
            .collect()
    }
}
