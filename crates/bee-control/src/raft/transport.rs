use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::types::{NodeCommand, NodeId, RpcMessage};

pub struct Router {
    pub senders: HashMap<NodeId, mpsc::Sender<(NodeId, RpcMessage)>>,
}

#[derive(Clone)]
pub struct InMemoryTransport {
    self_id: NodeId,
    router: Arc<Router>,
    inbox: Arc<tokio::sync::Mutex<mpsc::Receiver<(NodeId, RpcMessage)>>>,
    cmd_inbox: Arc<tokio::sync::Mutex<mpsc::Receiver<NodeCommand>>>,
}

impl InMemoryTransport {
    pub fn new(
        self_id: NodeId,
        router: Arc<Router>,
        inbox: mpsc::Receiver<(NodeId, RpcMessage)>,
        cmd_inbox: mpsc::Receiver<NodeCommand>,
    ) -> Self {
        Self {
            self_id,
            router,
            inbox: Arc::new(tokio::sync::Mutex::new(inbox)),
            cmd_inbox: Arc::new(tokio::sync::Mutex::new(cmd_inbox)),
        }
    }

    pub fn self_id(&self) -> NodeId {
        self.self_id
    }

    pub async fn send(&self, target: NodeId, msg: RpcMessage) -> Result<(), &'static str> {
        let sender = self.router.senders.get(&target).ok_or("unknown peer")?;
        sender.send((self.self_id, msg)).await.map_err(|_| "send failed")
    }

    pub async fn recv_rpc(&self) -> Option<(NodeId, RpcMessage)> {
        let mut inbox = self.inbox.lock().await;
        inbox.recv().await
    }

    pub async fn recv_cmd(&self) -> Option<NodeCommand> {
        let mut inbox = self.cmd_inbox.lock().await;
        inbox.recv().await
    }
}

/// Abstract the 4 calls a `Node` makes on its transport:
/// `self_id`, `send`, `recv_rpc`, `recv_cmd`.
///
/// `InMemoryTransport` (above) satisfies this trait
/// via mpsc channels. `TcpTransport` (in `tcp.rs`)
/// satisfies it via `bee_transport::Listener` + per-peer
/// `Connection`s. The trait exists so `Node::new` can
/// accept either, and so future transports (QUIC, Unix
/// socket, ...) can plug in without touching the Raft
/// state machine.
#[async_trait]
pub trait NodeTransport: Send + Sync + 'static {
    fn self_id(&self) -> NodeId;

    /// Send `msg` to peer `target`. The Node is allowed
    /// to fire-and-forget (queue + spawn) — failures
    /// are logged but do not surface back to the
    /// caller; the Raft timeouts catch the resulting
    /// election churn.
    async fn send(&self, target: NodeId, msg: RpcMessage) -> Result<(), TransportError>;

    /// Receive the next inbound `RpcMessage` (with its
    /// source node). `None` on graceful shutdown.
    async fn recv_rpc(&self) -> Option<(NodeId, RpcMessage)>;

    /// Receive the next `NodeCommand` (a `Submit { op,
    /// reply }` or a `Shutdown`). `None` on graceful
    /// shutdown.
    async fn recv_cmd(&self) -> Option<NodeCommand>;

    /// S33.4: push a `NodeCommand` (typically
    /// `Submit { op, reply }`) into the local
    /// Node's command channel. The leader's
    /// AdminServer uses this to submit ops to
    /// the Raft log. The `TcpTransport` impl
    /// already has a `pub fn submit_command`;
    /// we just add it to the trait so the
    /// AdminServer can call it generically.
    async fn submit_command(&self, cmd: NodeCommand) -> Result<(), TransportError>;
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("unknown peer {0}")]
    UnknownPeer(NodeId),
    #[error("send channel closed for peer {0}")]
    PeerClosed(NodeId),
    #[error("io: {0}")]
    Io(String),
}

#[async_trait]
impl NodeTransport for InMemoryTransport {
    fn self_id(&self) -> NodeId {
        InMemoryTransport::self_id(self)
    }

    async fn send(&self, target: NodeId, msg: RpcMessage) -> Result<(), TransportError> {
        InMemoryTransport::send(self, target, msg)
            .await
            .map_err(|e| match e {
                "unknown peer" => TransportError::UnknownPeer(target),
                _ => TransportError::Io(e.to_string()),
            })
    }

    async fn recv_rpc(&self) -> Option<(NodeId, RpcMessage)> {
        InMemoryTransport::recv_rpc(self).await
    }

    async fn recv_cmd(&self) -> Option<NodeCommand> {
        InMemoryTransport::recv_cmd(self).await
    }

    async fn submit_command(&self, cmd: NodeCommand) -> Result<(), TransportError> {
        // S33.4 MVP: the in-memory transport's
        // command channel is shared with the
        // Node. We add a `cmd_tx: Sender<NodeCommand>`
        // field in Task 2; the impl here sends
        // through it. For Task 1 the impl is a
        // placeholder error (the S33.3 tests
        // don't exercise write-through-the-
        // transport).
        Err(TransportError::Io(
            "submit_command: in-memory transport not yet wired (S33.4 Task 2)"
                .to_string(),
        ))
    }
}
