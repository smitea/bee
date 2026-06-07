use std::collections::HashMap;
use std::sync::Arc;

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
