use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, Mutex, Notify};

use crate::control_plane::ControlPlaneStateMachine;
use crate::kv::{KVStateMachine, Op, TxnError};

use super::node::{Node, NodeConfig, NodeState};
use super::tcp::TcpTransport;
use super::transport::{InMemoryTransport, NodeTransport, Router};
use super::types::{NodeCommand, NodeId, Role, RpcMessage, Term, LogIndex};

#[derive(Debug, Clone)]
pub struct ClusterConfig {
    pub n: usize,
    pub base_election_timeout: Duration,
    pub heartbeat_interval: Duration,
    /// S33.1: per-node transport specs. If empty (the
    /// default), the constructor falls back to the
    /// all-in-memory behavior (today's `Cluster::new`
    /// path). If non-empty, each entry's `transport` is
    /// honored.
    pub nodes: Vec<NodeSpec>,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            n: 3,
            base_election_timeout: Duration::from_millis(800),
            heartbeat_interval: Duration::from_millis(100),
            nodes: Vec::new(), // empty → in-memory default
        }
    }
}

/// S33.1: per-node transport spec. The MVP supports
/// `None` (in-memory) and `Some(Tcp { .. })`. The Tcp
/// variant is wired in Task 3; this Task (Task 2) just
/// introduces the type and refuses `Some(...)` with a
/// panic so the existing `Cluster::new(ClusterConfig::default())`
/// path is unchanged.
#[derive(Clone, Debug)]
pub enum NodeTransportSpec {
    Tcp {
        bind_addr: std::net::SocketAddr,
        peers: Vec<(NodeId, std::net::SocketAddr)>,
    },
}

#[derive(Clone, Debug)]
pub struct NodeSpec {
    pub id: NodeId,
    /// `None` → build the in-memory `InMemoryTransport`
    /// (today's behavior). `Some(Tcp { ... })` → build
    /// a `TcpTransport` (Task 3).
    pub transport: Option<NodeTransportSpec>,
    /// Per-node config override (election timeout,
    /// heartbeat interval, etc.). `None` → fall back
    /// to `ClusterConfig`'s defaults.
    pub node_config: Option<super::node::NodeConfig>,
}

impl Default for NodeSpec {
    fn default() -> Self {
        Self {
            id: 0,
            transport: None,
            node_config: None,
        }
    }
}

/// S33.1: per-node config override resolution. If the
/// `NodeSpec` carries a `NodeConfig`, use it verbatim;
/// otherwise fall back to `ClusterConfig`'s defaults
/// plus a `node_offset_ms` of `(index * 100)ms` so the
/// staggered-election behavior of `Cluster::new` is
/// preserved.
fn spec_to_node_node_config(
    spec: &NodeSpec,
    config: &ClusterConfig,
    index: usize,
) -> NodeConfig {
    if let Some(nc) = &spec.node_config {
        return nc.clone();
    }
    NodeConfig {
        base_election_timeout: config.base_election_timeout,
        heartbeat_interval: config.heartbeat_interval,
        node_offset_ms: (index as u64) * 100,
    }
}

#[derive(Clone)]
pub struct ClusterNodeHandle {
    pub id: NodeId,
    pub state: Arc<Mutex<NodeState>>,
    pub kv: Arc<Mutex<KVStateMachine>>,
    pub cp: Arc<Mutex<ControlPlaneStateMachine>>,
}

#[derive(Clone)]
struct ClusterNodeSlot {
    handle: ClusterNodeHandle,
    cmd_tx: mpsc::Sender<NodeCommand>,
    task_done: Arc<Notify>,
    alive: Arc<AtomicBool>,
}

#[derive(Clone)]
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
            let (_, cmd_tx) = cmd_txs.remove(0);
            let transport: Arc<dyn NodeTransport> = Arc::new(
                InMemoryTransport::new(
                    id,
                    router.clone(),
                    rpc_rx,
                    cmd_rx,
                    cmd_tx.clone(),
                ),
            );
            let kv = Arc::new(Mutex::new(KVStateMachine::new()));
            let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new()));
            let node_config = NodeConfig {
                base_election_timeout: config.base_election_timeout,
                heartbeat_interval: config.heartbeat_interval,
                node_offset_ms: (i as u64) * 100,
            };
            let node = Node::new(id, peer_ids, transport, kv.clone(), cp.clone(), node_config);
            let state = node.state();
            // S33.4: the slot's `cmd_tx` is the
            // same channel the InMemoryTransport
            // uses. The slot uses it for
            // `shutdown_node` (which sends
            // `NodeCommand::Shutdown`); the
            // AdminServer uses the transport's
            // `submit_command` (which is the same
            // channel via the cloned `cmd_tx`).
            let slot_cmd_tx = cmd_tx;
            let task_done = Arc::new(Notify::new());
            let task_done_inner = task_done.clone();
            tokio::spawn(async move {
                let _ = node.run().await;
                task_done_inner.notify_one();
            });
            slots.push(ClusterNodeSlot {
                handle: ClusterNodeHandle { id, state, kv, cp },
                cmd_tx: slot_cmd_tx,
                task_done,
                alive: Arc::new(AtomicBool::new(true)),
            });
        }

        Self { slots }
    }

    /// S33.1: build a cluster with explicit per-node
    /// transport specs. If `config.nodes` is empty,
    /// behaves identically to `Cluster::new` (the
    /// all-in-memory path). If `config.nodes` is
    /// non-empty, each non-empty `NodeSpec::transport`
    /// is honored.
    pub async fn new_with_specs(config: ClusterConfig) -> Self {
        let specs: std::collections::HashMap<NodeId, NodeSpec> = config
            .nodes
            .iter()
            .cloned()
            .map(|s| (s.id, s))
            .collect();
        let any_tcp = specs.values().any(|s| s.transport.is_some());
        if !any_tcp {
            return Self::new(config).await;
        }
        Self::new_with_tcp(config, &specs).await
    }

    /// S33.1 Task 3: inlined slot-building loop that
    /// honors each `NodeSpec::transport`. Today only
    /// `NodeTransportSpec::Tcp { .. }` is supported;
    /// `None` (in-memory) is handled by `new_with_specs`
    /// short-circuiting to `Self::new` above.
    async fn new_with_tcp(
        config: ClusterConfig,
        specs: &std::collections::HashMap<NodeId, NodeSpec>,
    ) -> Self {
        // Two-phase boot: bind ALL listeners first,
        // then dial ALL peers. This is required
        // because each Node's peers include the
        // others; if we bound-and-dialed in a single
        // loop, node 1's dial of node 2 would race
        // with node 2's bind.
        let mut tcp_handles: HashMap<NodeId, Arc<TcpTransport>> = HashMap::new();
        let mut ids: Vec<NodeId> = specs.keys().copied().collect();
        ids.sort();
        // Phase 1: bind.
        for &id in &ids {
            let spec = &specs[&id];
            let transport = match &spec.transport {
                Some(NodeTransportSpec::Tcp { bind_addr, peers }) => {
                    let t = TcpTransport::bind(id, bind_addr.to_string())
                        .await
                        .expect("TcpTransport::bind");
                    let _ = peers; // peer list is consumed in phase 2
                    Arc::new(t)
                }
                None => panic!(
                    "Cluster::new_with_tcp called with at least one Tcp spec, \
                     but spec for id={id} is None — use Cluster::new for the \
                     in-memory path."
                ),
            };
            tcp_handles.insert(id, transport);
        }
        // Phase 2: dial. Every listener is up; dials
        // succeed on the first attempt.
        for &id in &ids {
            let spec = &specs[&id];
            if let Some(NodeTransportSpec::Tcp { peers, .. }) = &spec.transport {
                let peer_addrs: Vec<(NodeId, String)> = peers
                    .iter()
                    .map(|(pid, addr)| (*pid, addr.to_string()))
                    .collect();
                tcp_handles[&id]
                    .connect_peers(peer_addrs)
                    .await
                    .expect("TcpTransport::connect_peers");
            }
        }
        let mut slots: Vec<ClusterNodeSlot> = Vec::new();
        for (i, &id) in ids.iter().enumerate() {
            let tcp = tcp_handles[&id].clone();
            let peer_ids: Vec<NodeId> = ids.iter().copied().filter(|&x| x != id).collect();
            // We need a `cmd_tx` to push Submit / Shutdown
            // into this node's command channel. TcpTransport
            // owns its own mpsc; we get at it via a
            // dedicated channel. For MVP we spawn a tiny
            // forwarder task that copies from our
            // Slot's cmd_tx into TcpTransport's
            // submit_command.
            let (slot_cmd_tx, mut slot_cmd_rx) =
                mpsc::channel::<NodeCommand>(64);
            let tcp_for_forward = tcp.clone();
            tokio::spawn(async move {
                while let Some(cmd) = slot_cmd_rx.recv().await {
                    if tcp_for_forward
                        .submit_command(cmd)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let kv = Arc::new(Mutex::new(KVStateMachine::new()));
            let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new()));
            let node_config = spec_to_node_node_config(&specs[&id], &config, i);
            let node = Node::new(
                id,
                peer_ids,
                tcp as Arc<dyn NodeTransport>,
                kv.clone(),
                cp.clone(),
                node_config,
            );
            let state = node.state();
            let task_done = Arc::new(Notify::new());
            let task_done_inner = task_done.clone();
            tokio::spawn(async move {
                let _ = node.run().await;
                task_done_inner.notify_one();
            });
            slots.push(ClusterNodeSlot {
                handle: ClusterNodeHandle { id, state, kv, cp },
                cmd_tx: slot_cmd_tx,
                task_done,
                alive: Arc::new(AtomicBool::new(true)),
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

    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &ClusterNodeHandle)> {
        self.slots.iter().map(|s| (s.handle.id, &s.handle))
    }

    /// Whether the node's run-loop is still alive. False after `shutdown_node`.
    pub fn is_alive(&self, id: NodeId) -> bool {
        self.slots
            .iter()
            .find(|s| s.handle.id == id)
            .map(|s| s.alive.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    pub async fn leader(&self) -> Option<NodeId> {
        for slot in &self.slots {
            if !slot.alive.load(Ordering::SeqCst) {
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
            if !slot.alive.load(Ordering::SeqCst) {
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

    pub async fn wait_for_leader(&self, timeout: Duration) -> Option<NodeId> {
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
                if !slot.alive.load(Ordering::SeqCst) {
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

    pub async fn wait_for_cp_converge(
        &self,
        expected_jobs: usize,
        expected_tasks: usize,
        timeout: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let mut all_match = true;
            for slot in &self.slots {
                if !slot.alive.load(Ordering::SeqCst) {
                    continue;
                }
                let cp = slot.handle.cp.lock().await;
                if cp.job_count() != expected_jobs || cp.task_count() != expected_tasks {
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

    pub async fn submit(&self, target: NodeId, op: Op) -> Result<(), TxnError> {
        let slot = self.slots.iter().find(|s| s.handle.id == target).ok_or_else(|| {
            TxnError::Conflict {
                key: format!("unknown_node_{target}"),
                expected: None,
                actual: None,
            }
        })?;
        if !slot.alive.load(Ordering::SeqCst) {
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

    pub async fn shutdown_node(&self, id: NodeId) {
        let (alive, cmd_tx, task_done) = {
            let Some(slot) = self.slots.iter().find(|s| s.handle.id == id) else {
                return;
            };
            (
                slot.alive.clone(),
                slot.cmd_tx.clone(),
                slot.task_done.clone(),
            )
        };
        alive.store(false, Ordering::SeqCst);
        let _ = cmd_tx.send(NodeCommand::Shutdown).await;
        task_done.notified().await;
    }

    /// S33.1 Task 4: alias for `shutdown_node` that
    /// reads more naturally in multi-process tests
    /// ("simulate the worker process for node N being
    /// SIGKILL'd"). The semantics are identical to a
    /// graceful shutdown: the slot's run-loop exits,
    /// its transport is dropped, and the cluster's
    /// heartbeat logic will eventually time out the
    /// missing peer. For a real SIGKILL simulation
    /// (where the process is gone before the cluster
    /// can mark the slot down), see
    /// `scripts/kill-node.sh` (Task 14) which
    /// `kill -9`s the OS-level process and leaves the
    /// cluster to discover the failure via heartbeats.
    pub async fn simulate_process_crash(&self, id: NodeId) {
        self.shutdown_node(id).await;
    }
}
