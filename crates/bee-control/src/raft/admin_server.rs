//! Per-Node admin RPC server. Listens on a separate
//! `bee_transport::Listener` (a different port from
//! the Raft channel). Each accepted `Connection` is
//! demuxed by `Frame::message_type`; only
//! `MessageType::Admin` is accepted. The handler
//! dispatches to the `ControlPlane` / `KV` state
//! machines and replies with
//! `bincode::serialize(AdminResponse)`.
//!
//! The MVP serves every request locally (no
//! leader-forwarding). Reads only; a future commit
//! (S33.3) will add the leader-forwarding path for
//! writes (e.g. `bee deploy --target`).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bee_codec::{Frame, MessageType};
use bee_transport::{Connection, Listener};

use crate::control_plane::ControlPlaneStateMachine;
use crate::kv::KVStateMachine;
use crate::raft::admin_protocol::{
    AdminRequest, AdminResponse, ClusterMetricsDetail, JobDep, JobDetail, JobSummary,
    NodeMetricsSummary, TaskDiagDetail,
};
use crate::raft::types::NodeId;

/// S33.1: shutdown signal sender held by the
/// `AdminServer`. The accept loop `select!`s on it
/// to break out gracefully; we don't try to cancel
/// `Listener::accept()` itself (no public API).
pub struct AdminServer {
    addr: SocketAddr,
    alive: Arc<AtomicBool>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    _listener_handle: tokio::task::JoinHandle<()>,
}

impl AdminServer {
    /// Bind `addr` and start accepting admin RPC
    /// connections. Spawns one tokio task per accept.
    /// The KV and CP state machines are read under a
    /// brief lock per request.
    pub async fn start(
        addr: SocketAddr,
        kv: Arc<tokio::sync::Mutex<KVStateMachine>>,
        cp: Arc<tokio::sync::Mutex<ControlPlaneStateMachine>>,
        get_role: Arc<dyn Fn() -> String + Send + Sync>,
        get_term: Arc<dyn Fn() -> u64 + Send + Sync>,
        get_commit_index: Arc<dyn Fn() -> u64 + Send + Sync>,
        get_log_length: Arc<dyn Fn() -> usize + Send + Sync>,
        get_leader_id: Arc<dyn Fn() -> Option<NodeId> + Send + Sync>,
    ) -> Result<Self, String> {
        let listener = Listener::bind(&addr.to_string())
            .await
            .map_err(|e| format!("admin bind {addr}: {e}"))?;
        let bound_addr = listener.local_addr();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_for_task = alive.clone();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let listener_handle = tokio::spawn(async move {
            tokio::pin!(shutdown_rx);
            loop {
                if !alive_for_task.load(Ordering::SeqCst) {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = &mut shutdown_rx => break,
                    accept_res = listener.accept() => {
                        let conn = match accept_res {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                        let kv = kv.clone();
                        let cp = cp.clone();
                        let get_role = get_role.clone();
                        let get_term = get_term.clone();
                        let get_commit_index = get_commit_index.clone();
                        let get_log_length = get_log_length.clone();
                        let get_leader_id = get_leader_id.clone();
                        tokio::spawn(async move {
                            handle_admin_connection(
                                conn,
                                kv,
                                cp,
                                get_role,
                                get_term,
                                get_commit_index,
                                get_log_length,
                                get_leader_id,
                            )
                            .await;
                        });
                    }
                }
            }
        });

        Ok(Self {
            addr: bound_addr,
            alive,
            shutdown_tx: Some(shutdown_tx),
            _listener_handle: listener_handle,
        })
    }

    /// Graceful shutdown: flip the alive flag AND
    /// send the oneshot signal so the accept loop
    /// breaks out of its current `accept()` call.
    pub fn shutdown(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

/// Handle one admin connection. Demuxes by message
/// type; only `Admin` is processed.
async fn handle_admin_connection(
    mut conn: Connection,
    kv: Arc<tokio::sync::Mutex<KVStateMachine>>,
    cp: Arc<tokio::sync::Mutex<ControlPlaneStateMachine>>,
    get_role: Arc<dyn Fn() -> String + Send + Sync>,
    get_term: Arc<dyn Fn() -> u64 + Send + Sync>,
    get_commit_index: Arc<dyn Fn() -> u64 + Send + Sync>,
    get_log_length: Arc<dyn Fn() -> usize + Send + Sync>,
    get_leader_id: Arc<dyn Fn() -> Option<NodeId> + Send + Sync>,
) {
    loop {
        let frame = match conn.recv_frame().await {
            Ok(f) => f,
            Err(_) => return,
        };
        if frame.message_type != MessageType::Admin {
            // Non-admin frames on the admin port are
            // dropped silently (the Raft channel uses
            // a different port; misroutes are logged
            // by the client in S33.3).
            continue;
        }
        let request: AdminRequest = match bincode::deserialize(&frame.body) {
            Ok(r) => r,
            Err(e) => {
                let resp = AdminResponse::Error(format!("bincode: {e}"));
                send_response(&mut conn, &resp).await;
                continue;
            }
        };
        let response = dispatch(
            request,
            &cp,
            &kv,
            &get_role,
            &get_term,
            &get_commit_index,
            &get_log_length,
            &get_leader_id,
        )
        .await;
        send_response(&mut conn, &response).await;
    }
}

async fn send_response(conn: &mut Connection, resp: &AdminResponse) {
    let body = match bincode::serialize(resp) {
        Ok(b) => b,
        Err(_) => return, // AdminResponse is always serializable; ignore
    };
    let frame = Frame::new(MessageType::Admin, 0, body);
    let _ = conn.send_frame(&frame).await;
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    req: AdminRequest,
    cp: &Arc<tokio::sync::Mutex<ControlPlaneStateMachine>>,
    kv: &Arc<tokio::sync::Mutex<KVStateMachine>>,
    get_role: &Arc<dyn Fn() -> String + Send + Sync>,
    get_term: &Arc<dyn Fn() -> u64 + Send + Sync>,
    get_commit_index: &Arc<dyn Fn() -> u64 + Send + Sync>,
    get_log_length: &Arc<dyn Fn() -> usize + Send + Sync>,
    get_leader_id: &Arc<dyn Fn() -> Option<NodeId> + Send + Sync>,
) -> AdminResponse {
    match req {
        AdminRequest::Ping => AdminResponse::Pong,
        AdminRequest::ListJobs => {
            let cp = cp.lock().await;
            let summaries: Vec<JobSummary> = cp
                .list_jobs()
                .iter()
                .map(|j| JobSummary::from_record(&cp, j))
                .collect();
            AdminResponse::JobList(summaries)
        }
        AdminRequest::JobInspect(id) => {
            let cp = cp.lock().await;
            match cp.get_job(id) {
                None => AdminResponse::JobDetail(None),
                Some(j) => {
                    let tasks: Vec<_> = cp
                        .list_tasks()
                        .into_iter()
                        .filter(|t| t.job_id == id)
                        .collect();
                    let deps = j
                        .dependencies
                        .iter()
                        .map(|d| JobDep {
                            upstream_job: d.upstream_job,
                            stream: d.stream.clone(),
                        })
                        .collect();
                    AdminResponse::JobDetail(Some(JobDetail {
                        job_id: j.job_id,
                        dag_hash: j.dag_hash.clone(),
                        lifecycle: j.lifecycle,
                        owner_node: j.owner_node,
                        dependencies: deps,
                        tasks,
                    }))
                }
            }
        }
        AdminRequest::TaskDiagnostics(id) => {
            let cp = cp.lock().await;
            match cp.get_task(id) {
                None => AdminResponse::TaskDiag(None),
                Some(t) => AdminResponse::TaskDiag(Some(TaskDiagDetail::from(t))),
            }
        }
        AdminRequest::ClusterStatus => {
            let cp_locked = cp.lock().await;
            // For the MVP, the admin server only sees
            // its own Node's metrics. The leader's
            // view of the cluster is the source of
            // truth; a follow-up (S33.3) forwards the
            // request to the leader.
            let node_metrics = NodeMetricsSummary {
                id: 0, // TODO: pass the node's own id through the closure
                role: get_role(),
                commit_index: get_commit_index(),
                log_length: get_log_length(),
            };
            // Touch the cp lock so the compiler
            // doesn't warn about unused state in
            // MVP (it would be used once we add
            // peer-list reporting).
            let _job_count = cp_locked.job_count();
            AdminResponse::ClusterMetrics(ClusterMetricsDetail {
                nodes: vec![node_metrics],
                leader_id: get_leader_id(),
                term: get_term(),
                commit_index: get_commit_index(),
            })
        }
    }
}

/// Suppress unused warnings on the `kv` parameter
/// (kept in the signature for parity with the Raft
/// server, which reads KV state on writes).
#[allow(dead_code)]
fn _kv_used(_: &Arc<tokio::sync::Mutex<KVStateMachine>>) {}
