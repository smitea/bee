//! Single-Node Raft loop for the KV state machine.
//!
//! ## S06 (this slice)
//! - Single node, no replication. The "Raft loop" is a simple
//!   receive-command / apply-to-state-machine iteration.
//! - `RaftNode::apply(op)` applies synchronously and is the canonical
//!   entry point for tests and the smoke binary.
//! - `RaftNode::run(cmd_rx)` is the async, channel-based driver used by
//!   callers that want the loop to own the node and process commands
//!   arriving on a channel.
//! - `committed_index` is a monotonically increasing counter that S07+
//!   will replace with the real Raft log index.
//!
//! ## S07+ (forward-looking)
//! - Replace the apply path with a real Raft library (openraft / raft-rs
//!   per ADR-0001). The same `KVStateMachine` becomes the `StateMachine`
//!   impl; `apply` becomes the `apply_committed_entries` callback.
//! - Add leader election, log replication, snapshotting.

use tokio::sync::{mpsc, oneshot};

use crate::kv::{KVStateMachine, Op, TxnError};

pub struct RaftNode {
    sm: KVStateMachine,
    committed_index: u64,
}

impl Default for RaftNode {
    fn default() -> Self {
        Self::new()
    }
}

impl RaftNode {
    pub fn new() -> Self {
        Self {
            sm: KVStateMachine::new(),
            committed_index: 0,
        }
    }

    pub fn state_machine(&self) -> &KVStateMachine {
        &self.sm
    }

    pub fn committed_index(&self) -> u64 {
        self.committed_index
    }

    /// Synchronously apply an op to the state machine. For S06 single-node
    /// this is immediate; for S07+ this will block on majority commit.
    pub fn apply(&mut self, op: Op) -> Result<(), TxnError> {
        match op {
            Op::Put { key, value } => {
                self.sm.put(key, value);
            }
            Op::Del { key } => {
                self.sm.del(&key);
            }
            Op::Cas { key, expected, new } => {
                self.sm.cas_checked(&key, expected.as_deref(), new)?;
            }
            Op::Txn { ops } => {
                self.sm.txn(ops)?;
            }
        }
        self.committed_index += 1;
        Ok(())
    }

    /// Async channel-based driver. Consumes self, runs until `cmd_rx` closes.
    /// Each command's op is applied; the result (Ok or Err) is sent back
    /// on the optional reply channel.
    pub async fn run(mut self, mut cmd_rx: mpsc::Receiver<Command>) {
        while let Some(cmd) = cmd_rx.recv().await {
            let result = self.apply(cmd.op);
            if let Some(reply) = cmd.reply {
                let _ = reply.send(result);
            }
        }
    }
}

pub struct Command {
    pub op: Op,
    pub reply: Option<oneshot::Sender<Result<(), TxnError>>>,
}
