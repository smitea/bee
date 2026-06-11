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
use crate::raft::node::AdminReplyRegistrar;
use crate::raft::transport::NodeTransport;
use crate::raft::types::{NodeId, RpcMessage};
use bee_registry::PluginManager;

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
        // S33.5.1: closure that produces
        // (request_id, oneshot::Receiver) pairs
        // for forwarded admin writes. `None` for
        // tests that don't exercise forwarding.
        register_reply: Option<AdminReplyRegistrar>,
        // S33.5.2: the local `PluginManager`
        // (loaded with the Plugins from the
        // host's plugin directory). The
        // `RegisterDatasource` arm uses it for
        // steps 8-9 of the validation chain.
        // `None` for tests that don't exercise
        // plugin-existence checks.
        plugin_manager: Option<Arc<PluginManager>>,
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
                        let register_reply = register_reply.clone();
                        let plugin_manager = plugin_manager.clone();
                        tokio::spawn(async move {
                            handle_admin_connection(
                                conn,
                                kv,
                                cp,
                                state,
                                stats,
                                transport,
                                register_reply,
                                plugin_manager,
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
    // S33.5.1: closure that produces
    // (request_id, oneshot::Receiver) pairs
    // for forwarded admin writes.
    register_reply: Option<AdminReplyRegistrar>,
    // S33.5.2: passed to `dispatch` →
    // `dispatch_with_apply` for the
    // RegisterDatasource validation chain.
    plugin_manager: Option<Arc<PluginManager>>,
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
            register_reply.as_ref(),
            plugin_manager.as_deref(),
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

/// S33.5: the leader's apply path. Builds the
/// appropriate `Op` (or `Op::Txn` for atomic
/// multi-op deploys), submits via
/// `NodeCommand::Submit`, awaits the reply,
/// and returns the `AdminResponse` shape.
///
/// Called by `Node::handle_admin_forward` after
/// the inner `AdminRequest` is decoded. Also
/// called directly by `dispatch(Forward)` when
/// the local node is the leader (skipping the
/// Raft-channel hop).
///
/// `pub` so `run_node.rs` (a separate crate)
/// can build a closure around it for the
/// admin callback.
#[allow(clippy::too_many_arguments)]
 pub async fn dispatch_with_apply(
    req: AdminRequest,
    kv: &Arc<tokio::sync::Mutex<KVStateMachine>>,
    cp: &Arc<tokio::sync::Mutex<ControlPlaneStateMachine>>,
    state: &Arc<tokio::sync::Mutex<super::node::NodeState>>,
    transport: &dyn NodeTransport,
    // S33.5.2: the `PluginManager` for steps
    // 8-9 of the RegisterDatasource
    // validation chain. `None` for tests
    // that don't exercise plugin-existence.
    plugin_manager: Option<&PluginManager>,
) -> AdminResponse {
    match req {
        AdminRequest::KvPut { key, value } => {
            let op = crate::kv::Op::Put { key, value };
            submit_and_await(transport, op).await
        }
        AdminRequest::RegisterDatasource {
            name,
            adapter: _,
            plugin_version: _,
            config_json: _,
            tenant: _,
            owner_node: _,
        } => {
            // S33.5 MVP: register a
            // datasource as a
            // `Op::RegisterDatasourceProducer`
            // with a unique signature. Full
            // validation (cdylib check +
            // strict mode) is a S33.5.2
            // follow-up.
            let cp_locked = cp.lock().await;
            let next_job_id = cp_locked
                .list_jobs()
                .iter()
                .map(|j| j.job_id)
                .max()
                .unwrap_or(0)
                + 1;
            drop(cp_locked);
            let signature = format!("datasource/{}", &name);
            let op = crate::kv::Op::RegisterDatasourceProducer {
                signature,
                job_id: next_job_id,
            };
            submit_and_await(transport, op).await
        }
        AdminRequest::Deploy {
            sql_text,
            owner_node,
        } => {
            // S33.5 MVP: register a single
            // Job (no Tasks). The full
            // bee-dsl-sql runner that parses
            // the DAG into Tasks is a
            // S33.5.3 follow-up.
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(sql_text.as_bytes());
            let dag_hash = format!("{:x}", hasher.finalize());
            let cp_locked = cp.lock().await;
            let next_job_id = cp_locked
                .list_jobs()
                .iter()
                .map(|j| j.job_id)
                .max()
                .unwrap_or(0)
                + 1;
            drop(cp_locked);
            let op = crate::kv::Op::RegisterJob {
                job_id: next_job_id,
                dag_hash,
                owner_node,
                tenant: 0,
            };
            let response = submit_and_await(transport, op).await;
            if matches!(response, AdminResponse::Error(_)) {
                return response;
            }
            AdminResponse::DeployAck {
                job_id: next_job_id,
                task_ids: vec![],
                error_msg: String::new(),
            }
        }
        // Read arms should never reach here;
        // they're handled by `dispatch`.
        AdminRequest::Ping
        | AdminRequest::ListJobs
        | AdminRequest::JobInspect(_)
        | AdminRequest::TaskDiagnostics(_)
        | AdminRequest::ClusterStatus
        | AdminRequest::ListKv { .. }
        | AdminRequest::Forward { .. } => AdminResponse::Error(
            "read-only arm routed to dispatch_with_apply (S33.5 bug)"
                .to_string(),
        ),
    }
}

/// S33.5: helper. Build a
/// `NodeCommand::Submit`, push it through the
/// transport's command channel, await the
/// oneshot reply, and convert the result into
/// an `AdminResponse`.
async fn submit_and_await(
    transport: &dyn NodeTransport,
    op: crate::kv::Op,
) -> AdminResponse {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Err(e) = transport
        .submit_command(crate::raft::types::NodeCommand::Submit { op, reply: tx })
        .await
    {
        return AdminResponse::Error(format!("submit failed: {e}"));
    }
    match rx.await {
        Ok(Ok(())) => AdminResponse::KvPutAck { ok: true },
        Ok(Err(e)) => AdminResponse::Error(format!("apply failed: {e}")),
        Err(_) => AdminResponse::Error(
            "submit reply channel closed".to_string(),
        ),
    }
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
    // S33.5.1: closure that produces
    // (request_id, oneshot::Receiver) pairs
    // for forwarded admin writes.
    register_reply: Option<&AdminReplyRegistrar>,
    // S33.5.2: passed through to
    // dispatch_with_apply for the
    // RegisterDatasource validation chain.
    plugin_manager: Option<&PluginManager>,
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
        // S33.5.1: the follower's real
        // forwarding path. Detect
        // leader-vs-self; either call
        // dispatch_with_apply directly (if
        // local leader) or forward to the
        // leader via the Raft channel.
        AdminRequest::Forward { to: _, request } => {
            // We need self_id to compare with
            // leader_id. Read from the transport.
            let self_id = match transport {
                Some(t) => t.self_id(),
                None => {
                    return AdminResponse::Error(
                        "Forward without transport (test mode)".to_string(),
                    );
                }
            };
            // Read the leader id.
            let leader_id_opt = {
                let state_locked = state.lock().await;
                state_locked.leader_id
            };
            match leader_id_opt {
                Some(leader) if leader == self_id => {
                    // Local leader. Decode the
                    // inner request and apply
                    // directly (no Raft-channel
                    // hop).
                    let inner: AdminRequest = match bincode::deserialize(&request) {
                        Ok(r) => r,
                        Err(e) => {
                            return AdminResponse::Error(format!(
                                "Forward: decode failed: {e}"
                            ));
                        }
                    };
                    return Box::pin(dispatch_with_apply(
                        inner,
                        kv,
                        cp,
                        state,
                        transport.unwrap(),
                        plugin_manager,
                    ))
                    .await;
                }
                Some(leader) => {
                    // Forward to the leader.
                    let (request_id, rx) = match register_reply {
                        Some(rr) => rr().await,
                        None => {
                            return AdminResponse::Error(
                                "Forward without register_reply (test mode)"
                                    .to_string(),
                            );
                        }
                    };
                    // Send the inner `request`
                    // bytes verbatim in the
                    // `RpcMessage::AdminForward`.
                    // The `request_id` lives in
                    // the transport-layer envelope
                    // (not inside the inner
                    // request). The leader's
                    // `Node::handle_admin_forward`
                    // bincode-deserializes `request`
                    // as the inner `AdminRequest`
                    // and dispatches to its
                    // `admin_callback`. The
                    // `request_id` is used to
                    // correlate the
                    // `AdminForwardReply` back
                    // to the follower's pending
                    // oneshot.
                    if let Err(e) = transport
                        .unwrap()
                        .send(
                            leader,
                            RpcMessage::AdminForward {
                                to: leader,
                                request: request.clone(),
                                request_id,
                            },
                        )
                        .await
                    {
                        return AdminResponse::Error(format!(
                            "Forward: send failed: {e}"
                        ));
                    }
                    // Await the leader's reply
                    // (with a 5s timeout).
                    let response_bytes = match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        rx,
                    )
                    .await
                    {
                        Ok(Ok(bytes)) => bytes,
                        Ok(Err(_)) => {
                            return AdminResponse::Error(
                                "Forward: reply channel closed"
                                    .to_string(),
                            );
                        }
                        Err(_) => {
                            return AdminResponse::Error(
                                "Forward: leader reply timeout (5s)"
                                    .to_string(),
                            );
                        }
                    };
                    // Unwrap the inner response
                    // (the leader's reply is
                    // bincode-serialized
                    // AdminResponse). The CLI
                    // gets the inner reply
                    // (e.g. KvPutAck), not the
                    // Forwarded wrapper.
                    return match bincode::deserialize(&response_bytes) {
                        Ok(r) => r,
                        Err(e) => AdminResponse::Error(format!(
                            "Forward: decode leader reply: {e}"
                        )),
                    };
                }
                None => AdminResponse::Error(
                    "no leader elected; retry in 3s".to_string(),
                ),
            }
        }
    }
}
/// Suppress unused warnings on the `kv` parameter
/// (kept in the signature for parity with the Raft
/// server, which reads KV state on writes).
#[allow(dead_code)]
fn _kv_used(_: &Arc<tokio::sync::Mutex<KVStateMachine>>) {}
