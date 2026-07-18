//! KV state machine for the Bee Raft log.
//!
//! The KV store is a logical state machine that runs on every Raft node
//! (replicated via Raft consensus). It exposes a small CRUD-plus-transaction
//! API over opaque bincode-serializable values.
//!
//! ## Namespace convention (per ADR-0004)
//! - `state/task/{TaskId}/{state_name}` — per-Task state, owned by exactly
//!   one Task at a time. New owner reads the latest value on Migrating.
//! - `state/checkpoint/{TaskId}` — atomic `(state, saved_offset)` snapshot.
//!   Updated via `txn` so the offset is consistent with the state blob.
//!
//! The KV does NOT interpret value contents. The KV is intentionally generic
//! so Handler authors can build any stateful structure (ring buffers,
//! sorted maps, custom aggregations) on top via bincode.
//!
//! ## Atomicity
//! `cas` is a single-key compare-and-swap. `txn` is a list of ops applied
//! atomically: either all ops succeed, or the KV is left unchanged. `txn`
//! does not support nested transactions in MVP.
//!
//! S06 (this slice) implements single-node semantics. S07+ will wrap this
//! state machine in a real Raft state machine trait (e.g., openraft) so
//! the same apply path is replicated via consensus.

use std::collections::HashMap;

use crate::control_plane::DependencyRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxnError {
    Conflict {
        key: String,
        expected: Option<Vec<u8>>,
        actual: Option<Vec<u8>>,
    },
    NestedTxn,
    WrongSm,
}

impl std::fmt::Display for TxnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxnError::Conflict { key, .. } => write!(f, "conflict at key {key}"),
            TxnError::NestedTxn => write!(f, "nested transactions are not supported"),
            TxnError::WrongSm => write!(f, "op belongs on a different state machine"),
        }
    }
}

impl std::error::Error for TxnError {}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Op {
    Put { key: String, value: Vec<u8> },
    Del { key: String },
    Cas { key: String, expected: Option<Vec<u8>>, new: Vec<u8> },
    Txn { ops: Vec<Op> },
    RegisterJob {
        job_id: u32,
        dag_hash: String,
        owner_node: u32,
        /// S29: tenant namespace (`u16`; 0 = global per ADR-0010).
        /// MVP: struct field only; ACL check is 1.x.
        tenant: u16,
        /// S18: cross-Pipeline edges. A Job B with
        /// `CREATE VIEW v AS SELECT ... FROM a.output` (where
        /// `a` is JobId 1) gets `DependencyRecord { upstream_job: 1, stream: "output" }`.
        /// The orchestrator (`evaluate_job_state`) holds B in
        /// `WaitingForUpstream` until upstream Job 1 is `Running`.
        #[serde(default)]
        dependencies: Vec<DependencyRecord>,
    },
    RegisterTask {
        task_id: u32,
        job_id: u32,
        phase_id: u32,
        owner_node: u32,
        status: TaskStatus,
        /// S25: wall-clock millis when the Task was assigned to its
        /// current owner. The Rebalancer uses this to gate
        /// rebalance on the `min_task_age_secs` threshold — a Task
        /// that just landed is not eligible to be migrated.
        started_at_ms: u64,
    },
    UpdateTaskStatus { task_id: u32, new_status: TaskStatus },
    Heartbeat { node_id: u32, timestamp_ms: u64 },
    /// S12 Work-Stealing: a free Node requests ownership of an `Orphaned`
    /// Task. Atomic check-and-set on the Leader: if the task is still
    /// `Orphaned`, transition to `Migrating` and set `owner_node` to the
    /// thief. Otherwise, the op is a no-op (someone else won).
    StealTask { thief_node: u32, task_id: u32 },
    /// S17 Producer/Subscriber detection: register `(signature -> job_id)`
    /// in the ControlPlane SM. Idempotent — if a Producer already exists
    /// for `signature`, the existing entry is preserved (first writer
    /// wins). Subsequent deploys of the same Datasource become
    /// Subscribers pointing at the existing Producer.
    RegisterDatasourceProducer { signature: String, job_id: u32 },
    /// S18 cross-Pipeline dependencies: declare that `downstream_job`
    /// reads from `upstream_job`'s named output `stream`. Recorded on
    /// the downstream Job's `dependencies` list. Idempotent (same
    /// upstream+stream pair is preserved on re-apply).
    RegisterDependency {
        downstream_job: u32,
        upstream_job: u32,
        stream: String,
    },
    /// S18 Job lifecycle transitions. Applied as `last-writer-wins`
    /// for `job.lifecycle`. The orchestrator drives Pending →
    /// Scheduled → Running (or WaitingForUpstream → Running once deps
    /// are met).
    UpdateJobLifecycle { job_id: u32, state: JobLifecycleState },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TaskStatus {
    Pending,
    Scheduled,
    Running,
    Orphaned,
    Migrating,
    Revoked,
    Completed,
    Failed,
}

/// S18: high-level lifecycle of a Pipeline Job. The enum lives in
/// the `bee-types` sub-crate so `bee-runtime` (and any other
/// consumer that doesn't otherwise need `bee-control`) can use it
/// without creating a `bee-control ↔ bee-runtime` cycle. This
/// re-export keeps the historical `bee_control::kv::JobLifecycleState`
/// path working.
pub use bee_types::JobLifecycleState;

#[derive(Debug, Default, Clone)]
pub struct KVStateMachine {
    store: HashMap<String, Vec<u8>>,
}

impl KVStateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.store.get(key).cloned()
    }

    pub fn put(&mut self, key: String, value: Vec<u8>) {
        self.store.insert(key, value);
    }

    pub fn del(&mut self, key: &str) -> Option<Vec<u8>> {
        self.store.remove(key)
    }

    /// S33.2: O(n) prefix scan. Returns all
    /// `(key, value)` pairs whose key starts with
    /// `prefix`. For the 24h-soak key space
    /// (`soak/run_*/tick_*` × 288 ticks), this is
    /// < 1 ms in practice. **Read-only**: does
    /// not mutate state. The AdminServer holds the
    /// `tokio::sync::Mutex<KVStateMachine>` lock
    /// for the duration of the call (same lock the
    /// Raft apply loop uses), so the read is
    /// consistent with the latest committed entry.
    pub fn list(&self, prefix: &str) -> Vec<(String, Vec<u8>)> {
        self.store
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Compare-and-swap. `expected = None` means "key must not exist";
    /// `expected = Some(v)` means "key must exist with value v".
    /// Returns `Ok(())` on success, `Err(TxnError::Conflict)` on mismatch.
    pub fn cas_checked(
        &mut self,
        key: &str,
        expected: Option<&[u8]>,
        new: Vec<u8>,
    ) -> Result<(), TxnError> {
        let actual = self.store.get(key).map(|v| v.as_slice());
        let matches = match (actual, expected) {
            (Some(a), Some(e)) => a == e,
            (None, None) => true,
            _ => false,
        };
        if matches {
            self.store.insert(key.to_string(), new);
            Ok(())
        } else {
            Err(TxnError::Conflict {
                key: key.to_string(),
                expected: expected.map(|v| v.to_vec()),
                actual: actual.map(|v| v.to_vec()),
            })
        }
    }

    /// Boolean flavor of `cas_checked` per spec (`kv.cas(key, expected, new) -> bool`).
    pub fn cas(&mut self, key: &str, expected: Option<&[u8]>, new: Vec<u8>) -> bool {
        self.cas_checked(key, expected, new).is_ok()
    }

    /// Apply a single op. Convenience for the Raft apply loop.
    /// Returns `Err(TxnError::WrongSm)` for non-KV ops (which belong on
    /// the ControlPlane SM).
    pub fn apply_op(&mut self, op: &Op) -> Result<(), TxnError> {
        match op {
            Op::Put { key, value } => {
                self.put(key.clone(), value.clone());
                Ok(())
            }
            Op::Del { key } => {
                self.del(key);
                Ok(())
            }
            Op::Cas { key, expected, new } => {
                self.cas_checked(key, expected.as_deref(), new.clone())
            }
            Op::Txn { ops } => self.txn(ops.clone()),
                Op::RegisterJob { .. }
                | Op::RegisterTask { .. }
                | Op::UpdateTaskStatus { .. }
                | Op::Heartbeat { .. }
                | Op::StealTask { .. }
                | Op::RegisterDatasourceProducer { .. }
                | Op::RegisterDependency { .. }
                | Op::UpdateJobLifecycle { .. } => Err(TxnError::WrongSm),
        }
    }

    /// Apply a list of ops atomically. Either all ops are applied, or the
    /// state is unchanged. Nested transactions return NestedTxn. Txn can
    /// only contain KV ops; mixed-KV/ControlPlane txns are rejected.
    pub fn txn(&mut self, ops: Vec<Op>) -> Result<(), TxnError> {
        for op in &ops {
            match op {
                Op::Txn { .. } => return Err(TxnError::NestedTxn),
                Op::RegisterJob { .. }
                | Op::RegisterTask { .. }
                | Op::UpdateTaskStatus { .. }
                | Op::Heartbeat { .. }
                | Op::StealTask { .. }
                | Op::RegisterDatasourceProducer { .. }
                | Op::RegisterDependency { .. }
                | Op::UpdateJobLifecycle { .. } => return Err(TxnError::WrongSm),
                _ => {}
            }
        }

        for op in &ops {
            if let Op::Cas { key, expected, new: _ } = op {
                let actual = self.store.get(key).map(|v| v.as_slice());
                let matches = match (actual, expected.as_deref()) {
                    (Some(a), Some(e)) => a == e,
                    (None, None) => true,
                    _ => false,
                };
                if !matches {
                    return Err(TxnError::Conflict {
                        key: key.clone(),
                        expected: expected.clone(),
                        actual: actual.map(|v| v.to_vec()),
                    });
                }
            }
        }

        for op in ops {
            match op {
                Op::Put { key, value } => self.put(key, value),
                Op::Del { key } => {
                    self.del(&key);
                }
                Op::Cas { key, expected: _, new } => {
                    self.put(key, new);
                }
                Op::Txn { .. } => unreachable!("nested txn checked above"),
                Op::RegisterJob { .. }
                | Op::RegisterTask { .. }
                | Op::UpdateTaskStatus { .. }
                | Op::Heartbeat { .. }
                | Op::StealTask { .. }
                | Op::RegisterDatasourceProducer { .. }
                | Op::RegisterDependency { .. }
                | Op::UpdateJobLifecycle { .. } => unreachable!("non-KV op checked above"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_returns_matching_prefix() {
        let mut kv = KVStateMachine::new();
        kv.put("soak/run_1/tick_1".to_string(), b"a".to_vec());
        kv.put("soak/run_1/tick_2".to_string(), b"b".to_vec());
        kv.put("soak/run_2/tick_1".to_string(), b"c".to_vec());
        kv.put("other/x".to_string(), b"d".to_vec());
        let r1 = kv.list("soak/run_1/");
        assert_eq!(r1.len(), 2);
        let r2 = kv.list("soak/");
        assert_eq!(r2.len(), 3);
        let r3 = kv.list("nope/");
        assert!(r3.is_empty());
        // Full-key prefix returns just that one.
        let r4 = kv.list("soak/run_1/tick_1");
        assert_eq!(r4.len(), 1);
    }

    #[test]
    fn list_empty_kv_returns_empty() {
        let kv = KVStateMachine::new();
        assert!(kv.list("anything/").is_empty());
    }
}
