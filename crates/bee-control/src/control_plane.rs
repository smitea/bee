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

use crate::kv::{Op, TaskStatus, TxnError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    pub job_id: u32,
    pub dag_hash: String,
    pub owner_node: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    pub task_id: u32,
    pub job_id: u32,
    pub phase_id: u32,
    pub owner_node: u32,
    pub status: TaskStatus,
}

#[derive(Debug, Default, Clone)]
pub struct ControlPlaneStateMachine {
    jobs: HashMap<u32, JobRecord>,
    tasks: HashMap<u32, TaskRecord>,
}

impl ControlPlaneStateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_op(&mut self, op: &Op) -> Result<(), TxnError> {
        match op {
            Op::RegisterJob { job_id, dag_hash, owner_node } => {
                self.jobs.insert(
                    *job_id,
                    JobRecord {
                        job_id: *job_id,
                        dag_hash: dag_hash.clone(),
                        owner_node: *owner_node,
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
            } => {
                self.tasks.insert(
                    *task_id,
                    TaskRecord {
                        task_id: *task_id,
                        job_id: *job_id,
                        phase_id: *phase_id,
                        owner_node: *owner_node,
                        status: *status,
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

    pub fn list_tasks(&self) -> Vec<TaskRecord> {
        let mut v: Vec<TaskRecord> = self.tasks.values().cloned().collect();
        v.sort_by_key(|t| t.task_id);
        v
    }

    pub fn get_job(&self, job_id: u32) -> Option<&JobRecord> {
        self.jobs.get(&job_id)
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
}
