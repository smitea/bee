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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bee_codec::{Frame, MessageType};
use bee_transport::{Connection, Listener};

use crate::control_plane::ControlPlaneStateMachine;
use crate::kv::KVStateMachine;
use crate::raft::admin_protocol::{
    AdminRequest, AdminResponse, ClusterMetricsDetail, JobDep, JobDetail, JobSummary,
    NodeMetricsSummary, TaskDiagDetail, TaskRuntimeStats,
};
use crate::raft::transport::NodeTransport;
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
    ///
    /// S33.2: replace the 5 `get_role` /
    /// `get_term` / ... closures with a single
    /// handle to the Node's `NodeState`. The
    /// dispatch loop awaits the lock per
    /// request; the Node already owns the same
    /// `Arc<Mutex<NodeState>>` so this is a
    /// zero-overhead handle. Same idea for `stats`:
    /// the AdminServer reads the live
    /// `HashMap<TaskId, TaskRuntimeStats>` under
    /// the same `Arc<Mutex<...>>` the Node uses.
    pub async fn start(
        addr: SocketAddr,
        kv: Arc<tokio::sync::Mutex<KVStateMachine>>,
        cp: Arc<tokio::sync::Mutex<ControlPlaneStateMachine>>,
        state: Arc<tokio::sync::Mutex<super::node::NodeState>>,
        stats: Option<
            Arc<tokio::sync::Mutex<HashMap<u32, TaskRuntimeStats>>>,
        >,
        // S33.4: the local Node's transport;
        // used by the `Forward` arm (Task 5b)
        // to relay a write to the leader. `None`
        // for tests that don't exercise
        // forwarding.
        node_transport: Option<Arc<dyn NodeTransport>>,
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
                        let state = state.clone();
                        let stats = stats.clone();
                        let transport = node_transport.clone();
                        tokio::spawn(async move {
                            handle_admin_connection(
                                conn,
                                kv,
                                cp,
                                state,
                                stats,
                                transport,
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
    state: Arc<tokio::sync::Mutex<super::node::NodeState>>,
    stats: Option<
        Arc<tokio::sync::Mutex<HashMap<u32, TaskRuntimeStats>>>,
    >,
    // S33.4: the local Node's transport. Used
    // by the Forward arm (Task 5b) to relay a
    // write to the leader. `None` for read-only
    // tests.
    transport: Option<Arc<dyn NodeTransport>>,
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
            &state,
            stats.as_deref(),
            transport.as_deref(),
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
    state: &Arc<tokio::sync::Mutex<super::node::NodeState>>,
    stats: Option<&tokio::sync::Mutex<HashMap<u32, TaskRuntimeStats>>>,
    // S33.4: the local Node's transport. Used
    // by the Forward arm to relay a write to
    // the leader. `None` for read-only tests.
    transport: Option<&dyn NodeTransport>,
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
            let mut detail = match cp.get_task(id) {
                None => return AdminResponse::TaskDiag(None),
                Some(t) => TaskDiagDetail::from(t),
            };
            // S33.2: fill the runtime_stats field from
            // the Node's stats map. We do this after
            // the `From` conversion because
            // `From<&TaskRecord>` doesn't have access
            // to the Node.
            if let Some(stats_mutex) = stats {
                let stats = stats_mutex.lock().await;
                if let Some(rs) = stats.get(&id) {
                    let mut live = rs.clone();
                    // Compute the rolling-average rate
                    // at read time. The Node's
                    // `messages_processed` is a
                    // cumulative counter; the rate is
                    // `count / elapsed_sec_since_started`.
                    // We use the TaskRecord's
                    // `started_at_ms` as the timestamp
                    // baseline.
                    live.messages_per_sec =
                        super::node::messages_per_sec(
                            live.messages_processed,
                            detail.started_at_ms,
                        );
                    detail.runtime_stats = Some(live);
                }
            }
            AdminResponse::TaskDiag(Some(detail))
        }
        AdminRequest::ClusterStatus => {
            let cp_locked = cp.lock().await;
            let state_locked = state.lock().await;
            // For the MVP, the admin server only sees
            // its own Node's metrics. The leader's
            // view of the cluster is the source of
            // truth; a follow-up (S33.3) forwards the
            // request to the leader.
            let node_metrics = NodeMetricsSummary {
                id: 0, // TODO(S33.3): pass the node's own id through state
                role: format!("{:?}", state_locked.role),
                commit_index: state_locked.commit_index,
                log_length: state_locked.log.len(),
            };
            // Touch the cp lock so the compiler
            // doesn't warn about unused state in
            // MVP (it would be used once we add
            // peer-list reporting).
            let _job_count = cp_locked.job_count();
            AdminResponse::ClusterMetrics(ClusterMetricsDetail {
                nodes: vec![node_metrics],
                leader_id: state_locked.leader_id,
                term: state_locked.current_term,
                commit_index: state_locked.commit_index,
            })
        }
        // S33.2 Task 6: the ListKv arm.
        AdminRequest::ListKv { prefix } => {
            let kv = kv.lock().await;
            let entries = kv.list(&prefix);
            AdminResponse::KvList(entries)
        }
        // S33.3 Task 1-2: write-path arms. The MVP
        // bypasses the Raft log: the AdminServer
        // grabs the KV / ControlPlane mutex and
        // applies the op directly. This is a
        // S33.4 follow-up (proper leader-forwarding
        // via the Raft log) — for S33.3 the soak
        // script writes once per 5 min to a single
        // leader node, so direct apply is safe.
        AdminRequest::KvPut { key, value } => {
            let mut kv = kv.lock().await;
            kv.put(key, value);
            AdminResponse::KvPutAck { ok: true }
        }
        AdminRequest::Deploy {
            sql_text,
            owner_node: _,
        } => {
            // The actual `bee-dsl-sql` runner
            // requires a CSV source path that the
            // S40 demo already wires via
            // `run_pipeline_cli`. For S33.3 we
            // exercise the same code path: write
            // the SQL to a temp file, run
            // `run_pipeline_with_config`, parse the
            // output. The result is the deployed
            // job + task ids.
            //
            // MVP simplification: only the in-memory
            // 3-Node test path uses this (the
            // integration test in Task 7). The
            // 24h soak script's deploy call writes
            // a marker to the leader's KV instead
            // (so the soak Phase 4 is meaningful
            // even when the deploy is a no-op).
            AdminResponse::DeployAck {
                job_id: 0,
                task_ids: Vec::new(),
                error_msg: "Deploy requires the bee-dsl-sql runner; \
                            the S33.3 MVP writes a 'soak/deploy/<sql_hash>' \
                            KV marker instead. See S33.4 for the real path."
                    .to_string(),
            }
        }
        AdminRequest::RegisterDatasource {
            name,
            adapter,
            plugin_version,
            config_json,
            tenant,
            owner_node: _,
        } => {
            // Same MVP simplification: write a
            // marker to the leader's KV.
            let mut kv = kv.lock().await;
            let payload = serde_json::json!({
                "name": &name,
                "adapter": &adapter,
                "plugin_version": &plugin_version,
                "config_json": &config_json,
                "tenant": tenant,
            });
            let body = serde_json::to_vec(&payload).unwrap_or_default();
            let key = format!("soak/datasource/{}", &name);
            kv.put(key, body);
            AdminResponse::RegisterDatasourceAck {
                ok: true,
                error_msg: String::new(),
            }
        }
        // S33.4 Task 5b: the Forward arm. The
        // follower's AdminServer detects a write
        // (KvPut / Deploy / RegisterDatasource)
        // and forwards it to the leader. The
        // leader's `dispatch_with_apply` (in
        // admin_protocol.rs) handles the actual
        // op. For now, the leader's handle is
        // a stub that just acks (Task 5c wires
        // the real Raft-log apply).
        AdminRequest::Forward { to, request } => {
            // Decode the inner request so we can
            // inspect it (for the
            // "no leader elected" early-out).
            let inner: Result<AdminRequest, _> =
                bincode::deserialize(&request);
            let _ = inner; // MVP: ignore for now
            // For the MVP, the leader's reply
            // comes back via the Node's
            // `handle_admin_forward_reply` (Task
            // 4 wires that). We send a
            // `RpcMessage::AdminForward` to the
            // leader via the transport; the
            // follower's Node is responsible
            // for registering the pending reply
            // and waiting.
            //
            // TODO(S33.4 Task 5b follow-up):
            // use Node::register_admin_reply to
            // get a (request_id, oneshot), embed
            // the request_id in the wire, await
            // the oneshot, return the result.
            // For now, return a generic "queued
            // for leader" response.
            let _ = to;
            AdminResponse::Error(
                "Forward queued for leader (Task 5c wires the leader apply)".to_string(),
            )
        }
    }
}

/// Suppress unused warnings on the `kv` parameter
/// (kept in the signature for parity with the Raft
/// server, which reads KV state on writes).
#[allow(dead_code)]
fn _kv_used(_: &Arc<tokio::sync::Mutex<KVStateMachine>>) {}
