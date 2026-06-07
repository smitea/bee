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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxnError {
    Conflict {
        key: String,
        expected: Option<Vec<u8>>,
        actual: Option<Vec<u8>>,
    },
    NestedTxn,
}

impl std::fmt::Display for TxnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxnError::Conflict { key, .. } => write!(f, "conflict at key {key}"),
            TxnError::NestedTxn => write!(f, "nested transactions are not supported"),
        }
    }
}

impl std::error::Error for TxnError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Put { key: String, value: Vec<u8> },
    Del { key: String },
    Cas { key: String, expected: Option<Vec<u8>>, new: Vec<u8> },
    Txn { ops: Vec<Op> },
}

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

    /// Apply a list of ops atomically. Either all ops are applied, or the
    /// state is unchanged. Nested transactions return NestedTxn.
    pub fn txn(&mut self, ops: Vec<Op>) -> Result<(), TxnError> {
        for op in &ops {
            if let Op::Txn { .. } = op {
                return Err(TxnError::NestedTxn);
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
            }
        }
        Ok(())
    }
}
