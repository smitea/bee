//! S33.1: TCP transport for multi-node Raft.
//!
//! One process = one `TcpTransport`. The transport
//! owns a `bee_transport::Listener` on `bind_addr` and
//! spawns one accept loop. Each accepted `Connection`
//! is read in its own task; every decoded frame is
//! forwarded to the local `inbox` (a tokio mpsc). A
//! second mpsc carries `NodeCommand` (Submit/Shutdown)
//! from a client (CLI admin) to the local node.
//!
//! Outbound sends go through a per-peer mpsc that's
//! spawned in `connect_peers`. Each dial task reads
//! from a `mpsc::Sender<RpcMessage>` owned by the
//! local `Node` and writes one frame per RPC. If the
//! peer hangs up, the dial task exits and a future
//! `send` call returns `TransportError::PeerClosed`.
//! Reconnection is a S33.2 concern.
//!
//! Wire format: we reuse `Frame` from `bee-codec` with
//! `MessageType::DataPacket` for the RPC payload
//! (we don't have a dedicated Rpc variant yet — see
//! S33.2 ADR follow-up). The source NodeId is
//! bincode-serialized into the body as
//! `RpcEnvelope { from: NodeId, msg: RpcMessage }`.
//!
//! See ADR-0010 + S33.1 design spec
//! `docs/superpowers/specs/2026-06-10-s33-1-multinode-cluster-failover-design.md`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bee_codec::{Frame, MessageType};
use bee_transport::{Connection, Listener, TransportError as BeeTransportError};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use super::transport::{NodeTransport, TransportError};
use super::types::{NodeCommand, NodeId, RpcMessage};

/// Body envelope: source NodeId + RpcMessage.
#[derive(Serialize, Deserialize)]
struct RpcEnvelope {
    from: NodeId,
    msg: RpcMessage,
}

struct PeerLink {
    tx: mpsc::Sender<RpcMessage>,
    _dial_task: JoinHandle<()>,
}

pub struct TcpTransport {
    self_id: NodeId,
    bind_addr: String,
    peers: Arc<Mutex<HashMap<NodeId, PeerLink>>>,
    inbox_tx: mpsc::Sender<(NodeId, RpcMessage)>,
    inbox_rx: Arc<Mutex<mpsc::Receiver<(NodeId, RpcMessage)>>>,
    cmd_tx: mpsc::Sender<NodeCommand>,
    cmd_rx: Arc<Mutex<mpsc::Receiver<NodeCommand>>>,
    _accept_task: JoinHandle<()>,
}

impl TcpTransport {
    pub async fn bind(self_id: NodeId, bind_addr: String) -> Result<Self, TransportError> {
        let listener = Listener::bind(&bind_addr)
            .await
            .map_err(|e| TransportError::Io(format!("bind {}: {e}", bind_addr)))?;
        let (inbox_tx, inbox_rx) = mpsc::channel::<(NodeId, RpcMessage)>(256);
        let (cmd_tx, cmd_rx) = mpsc::channel::<NodeCommand>(64);
        let inbox_rx = Arc::new(Mutex::new(inbox_rx));
        let cmd_rx = Arc::new(Mutex::new(cmd_rx));
        let peers: Arc<Mutex<HashMap<NodeId, PeerLink>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let inbox_tx_for_accept = inbox_tx.clone();

        let accept_task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(conn) => {
                        let inbox_tx = inbox_tx_for_accept.clone();
                        tokio::spawn(async move {
                            if let Err(e) = read_peer_loop(conn, inbox_tx).await {
                                eprintln!("tcp: peer reader exited: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("tcp: accept failed: {e}");
                    }
                }
            }
        });

        Ok(Self {
            self_id,
            bind_addr,
            peers,
            inbox_tx,
            inbox_rx,
            cmd_tx,
            cmd_rx,
            _accept_task: accept_task,
        })
    }

    pub async fn connect_peers(
        &self,
        peers: impl IntoIterator<Item = (NodeId, String)>,
    ) -> Result<(), TransportError> {
        for (peer_id, peer_addr) in peers {
            let (tx, mut rx) = mpsc::channel::<RpcMessage>(128);
            let self_id = self.self_id;
            // Retry the initial dial: when a 3-node
            // cluster boots from a single process the
            // listeners come up sequentially, so the
            // first dial attempt on a peer whose
            // accept loop hasn't started yet will fail
            // with ECONNREFUSED. We retry a handful of
            // times (50ms × 20 = 1s total) before
            // giving up and registering a
            // PeerClosed-fated channel.
            let mut conn: Option<Connection> = None;
            for attempt in 0..20 {
                match Connection::connect(&peer_addr).await {
                    Ok(c) => {
                        conn = Some(c);
                        break;
                    }
                    Err(e) => {
                        if attempt == 19 {
                            eprintln!(
                                "tcp: dial {peer_id}@{peer_addr} failed after 20 \
                                 attempts: {e} (will surface as PeerClosed on first send)"
                            );
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
            let mut conn = match conn {
                Some(c) => c,
                None => {
                    self.peers.lock().await.insert(
                        peer_id,
                        PeerLink {
                            tx,
                            _dial_task: tokio::spawn(async move {}),
                        },
                    );
                    continue;
                }
            };
            let dial_task = tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    let envelope = RpcEnvelope { from: self_id, msg };
                    let body = match bincode::serialize(&envelope) {
                        Ok(b) => b,
                        Err(e) => {
                            eprintln!("tcp: bincode encode to {peer_id} failed: {e}");
                            break;
                        }
                    };
                    let frame = Frame::new(MessageType::DataPacket, 0, body);
                    if let Err(e) = conn.send_frame(&frame).await {
                        eprintln!("tcp: send_frame to {peer_id} failed: {e}");
                        break;
                    }
                }
            });
            self.peers.lock().await.insert(
                peer_id,
                PeerLink {
                    tx,
                    _dial_task: dial_task,
                },
            );
        }
        Ok(())
    }

    pub fn bind_addr(&self) -> &str {
        &self.bind_addr
    }

    pub async fn submit_command(&self, cmd: NodeCommand) -> Result<(), TransportError> {
        self.cmd_tx
            .send(cmd)
            .await
            .map_err(|_| TransportError::Io("command channel closed".to_string()))
    }
}

async fn read_peer_loop(
    mut conn: Connection,
    inbox_tx: mpsc::Sender<(NodeId, RpcMessage)>,
) -> Result<(), String> {
    loop {
        let frame = conn
            .recv_frame()
            .await
            .map_err(|e| format!("recv_frame: {e}"))?;
        let env: RpcEnvelope = bincode::deserialize(&frame.body)
            .map_err(|e| format!("bincode decode: {e}"))?;
        if inbox_tx.send((env.from, env.msg)).await.is_err() {
            return Ok(());
        }
    }
}

#[async_trait]
impl NodeTransport for TcpTransport {
    fn self_id(&self) -> NodeId {
        self.self_id
    }

    async fn send(&self, target: NodeId, msg: RpcMessage) -> Result<(), TransportError> {
        let peers = self.peers.lock().await;
        let link = peers
            .get(&target)
            .ok_or(TransportError::UnknownPeer(target))?;
        link.tx
            .send(msg)
            .await
            .map_err(|_| TransportError::PeerClosed(target))
    }

    async fn recv_rpc(&self) -> Option<(NodeId, RpcMessage)> {
        let mut rx = self.inbox_rx.lock().await;
        rx.recv().await
    }

    async fn recv_cmd(&self) -> Option<NodeCommand> {
        let mut rx = self.cmd_rx.lock().await;
        rx.recv().await
    }

    async fn submit_command(&self, cmd: NodeCommand) -> Result<(), TransportError> {
        // Reuse the existing `pub fn submit_command`.
        // Translate the bee-transport error
        // type to our `TransportError` so the
        // trait signature is uniform.
        self.submit_command(cmd).await.map_err(|e| {
            TransportError::Io(format!("tcp submit_command: {e:?}"))
        })
    }
}

// silence unused import if the codec API changes shape during S33.x
#[allow(unused_imports)]
use BeeTransportError as _BeeTransportError;
