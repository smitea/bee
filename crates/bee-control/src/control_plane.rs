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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// S18: wall-clock millis when the Job was assigned to its
    /// current owner. The Rebalancer uses this to gate
    /// rebalance on the `min_task_age_secs` threshold.
    pub started_at_ms: u64,
    /// S28: when the Task is in `Migrating` status, this is the
    /// `owner_node` it had BEFORE the S12 StealTask transition
    /// (i.e., the source of the migration). The `bee diagnostics`
    /// CLI uses this to show the "source → target" view required
    /// by S28 acceptance. `None` for non-Migrating Tasks.
    pub migrating_from_node: Option<u32>,
    /// S29: tenant namespace (`u16`; 0 = global per ADR-0010).
    /// MVP: struct field only; ACL check
    /// (`ds.tenant == job.tenant || ds.tenant == 0`) is 1.x.
    pub tenant: u16,
}

/// S18: one upstream dependency declared by a downstream Job — for
/// example, "Job B reads from Job A's `output` stream". Resolved
/// at deploy time: same Node → in-process edge; different Node →
/// BRP data channel subscription. S18 stops at the metadata
/// layer; the actual data-channel resolution is the S25
/// cross-Node rebalance machinery.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DependencyRecord {
    pub upstream_job: u32,
    pub stream: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// S28: when the Task is in `Migrating` status, this is the
    /// `owner_node` it had BEFORE the S12 StealTask transition
    /// (i.e., the source of the migration). The `bee diagnostics`
    /// CLI uses this to show the "source → target" view required
    /// by S28 acceptance. `None` for non-Migrating Tasks.
    pub migrating_from_node: Option<u32>,
    /// S27: intra-Job Task dependencies (Phase DAG edges). A
    /// Task whose `phase_id` is downstream of another Task in
    /// the same Job lists the upstream's `task_id` here. S18
    /// (cross-Pipeline edges) is the follow-up that populates
    /// this in production for cross-Job edges; the MVP DAG
    /// always has empty `dependencies` (independent phases).
    /// `format_dag` (S27) reads this to render the DAG layout.
    #[serde(default)]
    pub dependencies: Vec<u32>,
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
    /// S24: per-Task metrics (events_processed_total, latency
    /// histogram, backpressure_wait_seconds). The worker pushes
    /// via `Op::RecordTaskMetrics`; `format_task_diagnostics`
    /// reads from this map. Last-writer-wins.
    task_metrics: HashMap<u32, crate::kv::TaskMetricsSnapshot>,
}

/// S17 §4: a Job's role with respect to Stream sharing.
/// - `Producer`: this JobId appears in the
///   `datasource_producers` registry (it is the canonical owner
///   of a Stream).
/// - `Subscriber`: this Job has at least one dependency whose
///   `upstream_job` is a Producer (it consumes a Stream owned by
///   another Job).
/// - `Independent`: neither — the Job is a normal, self-contained
///   Pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMode {
    Producer,
    Subscriber,
    Independent,
}

impl ControlPlaneStateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_op(&mut self, op: &Op) -> Result<(), TxnError> {
        match op {
            Op::RegisterJob { job_id, dag_hash, owner_node, tenant, dependencies } => {
                // Preserve existing lifecycle + dependencies +
                // started_at + migrating_from on a re-Register of the
                // same job_id (the Deployer may re-register after a
                // dep change).
                let (lifecycle, dependencies, started_at_ms, migrating_from_node) = self
                    .jobs
                    .get(job_id)
                    .map(|j| (j.lifecycle, j.dependencies.clone(), j.started_at_ms, j.migrating_from_node))
                    .unwrap_or((JobLifecycleState::Pending, Vec::new(), 0, None));
                self.jobs.insert(
                    *job_id,
                    JobRecord {
                        job_id: *job_id,
                        dag_hash: dag_hash.clone(),
                        owner_node: *owner_node,
                        lifecycle,
                        dependencies,
                        started_at_ms,
                        migrating_from_node,
                        tenant: *tenant,
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
                        migrating_from_node: None,
                        dependencies: Vec::new(),
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
                // S28: capture the source node BEFORE overwriting
                // owner_node, so `bee diagnostics` can show the
                // "source → target" view.
                task.migrating_from_node = Some(task.owner_node);
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
                // S21 TODO: when `state` transitions to a terminal
                // state (Completed / Failed / Revoked), call
                // `plugin_manager.release(plugin_id)` for each
                // plugin the Job was using. This wires the existing
                // `release` auto-unload logic into the Job-stop
                // path. The control plane currently doesn't own a
                // PluginManager reference; the orchestrator (S18
                // follow-up) will own one and dispatch the release
                // alongside UpdateJobLifecycle.
                Ok(())
            }
            // S24: worker pushes a metrics snapshot for a Task it's
            // running. Last-writer-wins; the most recent snapshot
            // is the source of truth for `format_task_diagnostics`.
            Op::RecordTaskMetrics { task_id, snapshot } => {
                self.task_metrics.insert(*task_id, snapshot.clone());
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

    /// S17 §3: when a Producer Job dies (Failed / Completed /
    /// removed), all Subscribers (Jobs whose `dependencies`
    /// list contains `upstream_job == producer_job_id`) must flip
    /// from `Running` to `WaitingForUpstream`. Returns the list of
    /// JobIds that were flipped, for the orchestrator's log.
    ///
    /// Idempotent: subscribers that are already
    /// `WaitingForUpstream` (or any non-`Running` state) are not
    /// touched, even if they still have a dependency on the dead
    /// producer.
    pub fn propagate_producer_death(
        &mut self,
        producer_job_id: u32,
    ) -> Vec<u32> {
        let mut flipped = vec![];
        for (job_id, job) in self.jobs.iter_mut() {
            let depends_on_dead = job
                .dependencies
                .iter()
                .any(|d| d.upstream_job == producer_job_id);
            if depends_on_dead
                && job.lifecycle == JobLifecycleState::Running
            {
                job.lifecycle = JobLifecycleState::WaitingForUpstream;
                flipped.push(*job_id);
            }
        }
        flipped
    }

    /// S17 §4: derive a Job's [`JobMode`] at view time. A JobId
    /// with no record returns `Independent` (defensive default
    /// for views that may be queried before all jobs are
    /// registered).
    pub fn job_mode(&self, job_id: u32) -> JobMode {
        if self.datasource_producers.values().any(|&p| p == job_id) {
            return JobMode::Producer;
        }
        if let Some(job) = self.jobs.get(&job_id) {
            for d in &job.dependencies {
                if self.datasource_producers.values().any(|&p| p == d.upstream_job) {
                    return JobMode::Subscriber;
                }
            }
        }
        JobMode::Independent
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

    /// S24: get the most recent metrics snapshot for a Task.
    /// Returns `None` if the worker hasn't pushed a snapshot yet.
    pub fn get_task_metrics(
        &self,
        task_id: u32,
    ) -> Option<&crate::kv::TaskMetricsSnapshot> {
        self.task_metrics.get(&task_id)
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
