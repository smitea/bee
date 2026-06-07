use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::kv::{KVStateMachine, Op, TxnError};

use super::node::{Node, NodeConfig, NodeState};
use super::transport::{InMemoryTransport, Router};
use super::types::{NodeCommand, NodeId, Role, RpcMessage, Term, LogIndex};

#[derive(Debug, Clone)]
pub struct ClusterConfig {
    pub n: usize,
    pub base_election_timeout: Duration,
    pub heartbeat_interval: Duration,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            n: 3,
            base_election_timeout: Duration::from_millis(800),
            heartbeat_interval: Duration::from_millis(100),
        }
    }
}

pub struct ClusterNodeHandle {
    pub id: NodeId,
    pub state: Arc<Mutex<NodeState>>,
    pub kv: Arc<Mutex<KVStateMachine>>,
}

struct ClusterNodeSlot {
    handle: ClusterNodeHandle,
    cmd_tx: mpsc::Sender<NodeCommand>,
    task: Option<JoinHandle<()>>,
    alive: bool,
}

pub struct Cluster {
    slots: Vec<ClusterNodeSlot>,
}

#[derive(Debug, Clone)]
pub struct NodeMetrics {
    pub id: NodeId,
    pub role: Role,
    pub term: Term,
    pub commit_index: LogIndex,
    pub log_length: usize,
    pub leader_id: Option<NodeId>,
}

impl Cluster {
    pub async fn new(config: ClusterConfig) -> Self {
        let n = config.n;
        let ids: Vec<NodeId> = (1..=n as NodeId).collect();

        let mut senders: HashMap<NodeId, mpsc::Sender<(NodeId, RpcMessage)>> = HashMap::new();
        let mut inboxes: Vec<(NodeId, mpsc::Receiver<(NodeId, RpcMessage)>)> = Vec::new();
        let mut cmd_inboxes: Vec<(NodeId, mpsc::Receiver<NodeCommand>)> = Vec::new();
        let mut cmd_txs: Vec<(NodeId, mpsc::Sender<NodeCommand>)> = Vec::new();
        for &id in &ids {
            let (tx, rx) = mpsc::channel(128);
            senders.insert(id, tx);
            inboxes.push((id, rx));
            let (ctx, crx) = mpsc::channel(128);
            cmd_txs.push((id, ctx));
            cmd_inboxes.push((id, crx));
        }
        let router = Arc::new(Router { senders });

        let mut slots: Vec<ClusterNodeSlot> = Vec::new();
        for (i, &id) in ids.iter().enumerate() {
            let peer_ids: Vec<NodeId> = ids.iter().copied().filter(|&x| x != id).collect();
            let (_, rpc_rx) = inboxes.remove(0);
            let (_, cmd_rx) = cmd_inboxes.remove(0);
            let transport = InMemoryTransport::new(id, router.clone(), rpc_rx, cmd_rx);
            let kv = Arc::new(Mutex::new(KVStateMachine::new()));
            let node_config = NodeConfig {
                base_election_timeout: config.base_election_timeout,
                heartbeat_interval: config.heartbeat_interval,
                node_offset_ms: (i as u64) * 100,
            };
            let node = Node::new(id, peer_ids, transport, kv.clone(), node_config);
            let state = node.state();
            let (_, ctx) = cmd_txs.remove(0);
            let task = tokio::spawn(node.run());
            slots.push(ClusterNodeSlot {
                handle: ClusterNodeHandle { id, state, kv },
                cmd_tx: ctx,
                task: Some(task),
                alive: true,
            });
        }

        Self { slots }
    }

    pub fn node(&self, id: NodeId) -> Option<&ClusterNodeHandle> {
        self.slots
            .iter()
            .find(|s| s.handle.id == id)
            .map(|s| &s.handle)
    }

    pub async fn leader(&self) -> Option<NodeId> {
        for slot in &self.slots {
            if !slot.alive {
                continue;
            }
            let state = slot.handle.state.lock().await;
            if state.role == Role::Leader {
                return Some(slot.handle.id);
            }
        }
        None
    }

    pub async fn metrics(&self) -> Vec<NodeMetrics> {
        let mut out = Vec::new();
        for slot in &self.slots {
            if !slot.alive {
                continue;
            }
            let state = slot.handle.state.lock().await;
            out.push(NodeMetrics {
                id: slot.handle.id,
                role: state.role,
                term: state.current_term,
                commit_index: state.commit_index,
                log_length: state.log.len(),
                leader_id: state.leader_id,
            });
        }
        out
    }

    pub async fn wait_for_leader(
        &self,
        timeout: Duration,
    ) -> Option<NodeId> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(l) = self.leader().await {
                return Some(l);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn wait_for_log_converge(
        &self,
        key: &str,
        expected: &[u8],
        timeout: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let mut all_match = true;
            for slot in &self.slots {
                if !slot.alive {
                    continue;
                }
                let kv = slot.handle.kv.lock().await;
                if kv.get(key).as_deref() != Some(expected) {
                    all_match = false;
                    break;
                }
            }
            if all_match {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn submit(
        &self,
        target: NodeId,
        op: Op,
    ) -> Result<(), TxnError> {
        let slot = self.slots.iter().find(|s| s.handle.id == target).ok_or_else(|| {
            TxnError::Conflict {
                key: format!("unknown_node_{target}"),
                expected: None,
                actual: None,
            }
        })?;
        if !slot.alive {
            return Err(TxnError::Conflict {
                key: format!("node_{target}_not_running"),
                expected: None,
                actual: None,
            });
        }
        let (tx, rx) = oneshot::channel();
        slot.cmd_tx
            .send(NodeCommand::Submit { op, reply: tx })
            .await
            .map_err(|_| TxnError::Conflict {
                key: format!("node_{target}_not_running"),
                expected: None,
                actual: None,
            })?;
        rx.await.map_err(|_| TxnError::Conflict {
            key: format!("node_{target}_reply_dropped"),
            expected: None,
            actual: None,
        })?
    }

    pub async fn shutdown_node(&mut self, id: NodeId) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.handle.id == id) {
            slot.alive = false;
            let _ = slot.cmd_tx.send(NodeCommand::Shutdown).await;
            if let Some(task) = slot.task.take() {
                let _ = task.await;
            }
        }
    }
}
