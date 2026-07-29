//! Bee GUI Tauri backend.
//!
//! Wraps `bee_control::raft::AdminClient` and exposes Tauri commands
//! to the React frontend. For MVP, single connection (mirrors the
//! S-1a "single AdminClient" decision); S-Tauri.x will add multi-cluster.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bee_control::raft::admin_client::{AdminClient, AdminError};
use bee_control::raft::{AdminRequest, AdminResponse};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Error(String),
    Disconnected,
}

impl ConnectionState {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Error(_) => "Error",
            Self::Disconnected => "Disconnected",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConnectionMsg {
    StateChanged(ConnectionState),
    CallResult {
        id: u64,
        result: Result<AdminResponse, String>,
    },
}

#[derive(Debug)]
#[allow(dead_code)] // Shutdown is reserved for S-Tauri.x admin-quit command
enum Cmd {
    Call {
        id: u64,
        req: AdminRequest,
        reply: oneshot::Sender<Result<AdminResponse, String>>,
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
    /// Async send to the connection thread. Safe to call from a tokio
    /// runtime (e.g. inside a `#[tauri::command]`).
    pub async fn call(
        &self,
        req: AdminRequest,
    ) -> Result<oneshot::Receiver<Result<AdminResponse, String>>, String> {
        let (reply, rx) = oneshot::channel();
        let id = next_id();
        self.cmd_tx
            .send(Cmd::Call { id, req, reply })
            .await
            .map_err(|e| format!("send: {e}"))?;
        Ok(rx)
    }
}

fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static ID: AtomicU64 = AtomicU64::new(1);
    ID.fetch_add(1, Ordering::Relaxed)
}

/// Bundle returned by `spawn`: the handle + the receiver half of the
/// ConnectionMsg mpsc. The App owns the receiver and drains it each tick.
pub struct ConnectionBundle {
    pub handle: ConnectionHandle,
    pub receiver: mpsc::Receiver<ConnectionMsg>,
}

pub fn spawn(addr: SocketAddr) -> ConnectionBundle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Cmd>(64);
    let (msg_tx, msg_rx) = mpsc::channel::<ConnectionMsg>(64);
    let state = Arc::new(Mutex::new(ConnectionState::Connecting));
    let state_clone = Arc::clone(&state);

    thread::Builder::new()
        .name("bee-gui-tauri-conn".to_string())
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
                            *state_clone.lock().unwrap() =
                                ConnectionState::Error(reason.clone());
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

async fn run_request_loop(
    mut client: AdminClient,
    cmd_rx: &mut mpsc::Receiver<Cmd>,
    msg_tx: &mpsc::Sender<ConnectionMsg>,
    _state: &Arc<Mutex<ConnectionState>>,
) {
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            Cmd::Shutdown => break,
            Cmd::Call { id, req, reply } => {
                let result = client.call(req).await.map_err(admin_to_gui);
                let _ = msg_tx
                    .send(ConnectionMsg::CallResult { id, result: result.clone() })
                    .await;
                let _ = reply.send(result);
            }
        }
    }
}

fn admin_to_gui(e: AdminError) -> String {
    match e {
        AdminError::Io(msg) => format!("io: {msg}"),
        AdminError::Bincode(msg) => format!("bincode: {msg}"),
        AdminError::ServerError(msg) => msg,
    }
}

/// Global single connection (MVP; S-Tauri.x adds multi).
static GLOBAL: Mutex<Option<ConnectionBundle>> = Mutex::new(None);

pub fn install_bundle(b: ConnectionBundle) {
    *GLOBAL.lock().unwrap() = Some(b);
}

pub fn with_handle<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&ConnectionHandle) -> Result<R, String>,
{
    let g = GLOBAL.lock().unwrap();
    let b = g.as_ref().ok_or_else(|| "no active connection".to_string())?;
    f(&b.handle)
}

pub fn addr_parse(s: &str) -> Result<SocketAddr, String> {
    s.parse().map_err(|e: std::net::AddrParseError| e.to_string())
}