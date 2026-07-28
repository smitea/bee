//! Single AdminClient connection lifecycle:
//!   Connecting → Connected → Error(reason) → Disconnected
//! Spawned on its own std::thread + tokio runtime. Communicates with the
//! iced main thread via a held `mpsc::Receiver<ConnectionMsg>`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bee_control::raft::admin_client::{AdminClient, AdminError};
use bee_control::raft::{AdminRequest, AdminResponse};
use tokio::sync::{mpsc, oneshot};

use crate::error::{log_rpc_failure, CallContext, GuiError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Error(String),
    Disconnected,
}

impl ConnectionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Error(_) => "Error",
            Self::Disconnected => "Disconnected",
        }
    }

    fn tag(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Error(_) => "Error",
            Self::Disconnected => "Disconnected",
        }
    }
}

#[derive(Debug)]
pub enum ConnectionMsg {
    StateChanged(ConnectionState),
    CallResult {
        id: u64,
        result: Result<AdminResponse, GuiError>,
    },
}

#[derive(Debug)]
pub enum Cmd {
    Call {
        id: u64,
        req: AdminRequest,
        reply: oneshot::Sender<Result<AdminResponse, GuiError>>,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct ConnectionHandle {
    addr: SocketAddr,
    state: Arc<Mutex<ConnectionState>>,
    cmd_tx: mpsc::Sender<Cmd>,
}

impl ConnectionHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn state(&self) -> ConnectionState {
        self.state.lock().unwrap().clone()
    }

    pub fn call(&self, req: AdminRequest) -> oneshot::Receiver<Result<AdminResponse, GuiError>> {
        let (reply, rx) = oneshot::channel();
        let id = next_id();
        let cmd = Cmd::Call { id, req, reply };
        let _ = self.cmd_tx.blocking_send(cmd);
        rx
    }

    pub fn shutdown(&self) {
        let _ = self.cmd_tx.blocking_send(Cmd::Shutdown);
    }
}

fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static ID: AtomicU64 = AtomicU64::new(1);
    ID.fetch_add(1, Ordering::Relaxed)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Bundle returned by `spawn`: the handle + the receiver half of the
/// `ConnectionMsg` mpsc. The App owns the receiver and drains it each
/// `update`.
pub struct ConnectionBundle {
    pub handle: ConnectionHandle,
    pub receiver: mpsc::Receiver<ConnectionMsg>,
}

/// Spawn the tokio runtime thread.
pub fn spawn(addr: SocketAddr) -> ConnectionBundle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Cmd>(64);
    let (msg_tx, msg_rx) = mpsc::channel::<ConnectionMsg>(64);
    let state = Arc::new(Mutex::new(ConnectionState::Connecting));
    let state_clone = Arc::clone(&state);

    thread::Builder::new()
        .name("bee-gui-conn".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let _ = msg_tx
                    .send(ConnectionMsg::StateChanged(ConnectionState::Connecting))
                    .await;

                loop {
                    match AdminClient::connect(addr).await {
                        Ok(mut client) => {
                            *state_clone.lock().unwrap() = ConnectionState::Connected;
                            let _ = msg_tx
                                .send(ConnectionMsg::StateChanged(ConnectionState::Connected))
                                .await;
                            run_request_loop(client, &mut cmd_rx, &msg_tx, &state_clone).await;
                        }
                        Err(e) => {
                            let reason = format!("connect failed: {}", e);
                            *state_clone.lock().unwrap() = ConnectionState::Error(reason.clone());
                            let _ = msg_tx
                                .send(ConnectionMsg::StateChanged(ConnectionState::Error(reason)))
                                .await;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            });
        })
        .expect("spawn conn thread");

    let handle = ConnectionHandle {
        addr,
        state,
        cmd_tx,
    };
    ConnectionBundle {
        handle,
        receiver: msg_rx,
    }
}

/// Non-blocking drain of pending `ConnectionMsg`s. Returns `Vec` so the
/// App can re-emit each as a `Message::Connection` via `update`.
pub fn try_drain(rx: &mut mpsc::Receiver<ConnectionMsg>) -> Vec<ConnectionMsg> {
    use tokio::sync::mpsc::error::TryRecvError;
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(m) => out.push(m),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        }
    }
    out
}

async fn run_request_loop(
    mut client: AdminClient,
    cmd_rx: &mut mpsc::Receiver<Cmd>,
    msg_tx: &mpsc::Sender<ConnectionMsg>,
    state: &Arc<Mutex<ConnectionState>>,
) {
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            Cmd::Shutdown => break,
            Cmd::Call { id, req, reply } => {
                let started_at_ms = now_ms();
                let rpc_kind = rpc_kind_of(&req);
                let result = client.call(req).await.map_err(admin_to_gui);
                let elapsed_ms = now_ms().saturating_sub(started_at_ms);
                let conn_state_tag = state.lock().unwrap().tag();
                let ctx = CallContext {
                    id,
                    rpc_kind,
                    addr: SocketAddr::from(([0, 0, 0, 0], 0)),
                    started_at_ms,
                    elapsed_ms,
                    attempt: 1,
                    conn_state: conn_state_tag,
                };
                if let Err(ref e) = result {
                    log_rpc_failure(&ctx, e);
                }
                let result_for_msg = clone_result(&result);
                let _ = msg_tx
                    .send(ConnectionMsg::CallResult { id, result: result_for_msg })
                    .await;
                let _ = reply.send(result);
            }
        }
    }
}

fn clone_result(r: &Result<AdminResponse, GuiError>) -> Result<AdminResponse, GuiError> {
    match r {
        Ok(v) => Ok(v.clone()),
        Err(e) => Err(match e {
            GuiError::Io { source } => GuiError::Io {
                source: std::io::Error::new(source.kind(), source.to_string()),
            },
            GuiError::Connect {
                addr,
                attempts,
                last_err,
            } => GuiError::Connect {
                addr: *addr,
                attempts: *attempts,
                last_err: last_err.clone(),
            },
            GuiError::Timeout { rpc, elapsed_ms } => GuiError::Timeout {
                rpc,
                elapsed_ms: *elapsed_ms,
            },
            GuiError::RpcServer { msg } => GuiError::RpcServer { msg: msg.clone() },
            GuiError::Wire { kind, detail } => GuiError::Wire {
                kind: *kind,
                detail: detail.clone(),
            },
            GuiError::ConnectionLost { last_seen_ms } => GuiError::ConnectionLost {
                last_seen_ms: *last_seen_ms,
            },
            GuiError::Cancelled => GuiError::Cancelled,
        }),
    }
}

fn admin_to_gui(e: AdminError) -> GuiError {
    match e {
        AdminError::Io(msg) => GuiError::Io {
            source: std::io::Error::new(std::io::ErrorKind::Other, msg),
        },
        AdminError::Bincode(msg) => GuiError::Wire {
            kind: crate::error::WireErrKind::Decode,
            detail: msg,
        },
        AdminError::ServerError(msg) => GuiError::RpcServer { msg },
    }
}

fn rpc_kind_of(req: &AdminRequest) -> &'static str {
    match req {
        AdminRequest::Ping => "Ping",
        AdminRequest::ClusterStatus => "ClusterStatus",
        AdminRequest::ListJobs => "ListJobs",
        AdminRequest::JobInspect(_) => "JobInspect",
        AdminRequest::TaskDiagnostics(_) => "TaskDiagnostics",
        AdminRequest::ListKv { .. } => "ListKv",
        AdminRequest::KvPut { .. } => "KvPut",
        AdminRequest::Deploy { .. } => "Deploy",
        AdminRequest::RegisterDatasource { .. } => "RegisterDatasource",
        AdminRequest::Forward { .. } => "Forward",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_state_machine() {
        let s = ConnectionState::Connecting;
        assert_eq!(s.as_str(), "Connecting");
        let s = ConnectionState::Connected;
        assert_eq!(s.as_str(), "Connected");
        let s = ConnectionState::Error("nope".to_string());
        assert_eq!(s.as_str(), "Error");
        let s = ConnectionState::Disconnected;
        assert_eq!(s.as_str(), "Disconnected");
    }

    #[test]
    fn connection_state_tag() {
        assert_eq!(ConnectionState::Connected.tag(), "Connected");
        assert_eq!(
            ConnectionState::Error("x".to_string()).tag(),
            "Error"
        );
    }

    #[test]
    fn rpc_kind_of_known_variants() {
        assert_eq!(rpc_kind_of(&AdminRequest::Ping), "Ping");
        assert_eq!(rpc_kind_of(&AdminRequest::ClusterStatus), "ClusterStatus");
        assert_eq!(rpc_kind_of(&AdminRequest::ListJobs), "ListJobs");
    }

    #[test]
    fn try_drain_empty() {
        let (_tx, mut rx) = mpsc::channel::<ConnectionMsg>(4);
        let v = try_drain(&mut rx);
        assert!(v.is_empty());
    }
}