//! `AdminRequest` / `AdminResponse` — the wire types
//! for the `bee --connect <addr>` admin RPC.
//!
//! Wire format: a `bee_transport::Frame` whose body is
//! `bincode::serialize(AdminRequest)` or
//! `bincode::serialize(AdminResponse)`. The admin
//! server (per-Node, in `admin_server.rs`) and the
//! admin client (in `admin_client.rs`) both speak
//! this format. The transport layer is just `Frame`;
//! the admin layer is the request/response shape.
//!
//! `MessageType::Admin` (a new value in
//! `MessageType`) distinguishes admin traffic from
//! `Data` traffic (which is the Raft RPC channel).
//! Adding a new MessageType variant is a one-liner
//! in `bee-codec`; see Task 6 for the actual change.

use serde::{Deserialize, Serialize};

use crate::control_plane::{ControlPlaneStateMachine, JobMode, JobRecord, TaskRecord};
use crate::kv::{JobLifecycleState, TaskStatus};
use crate::raft::types::RpcMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AdminRequest {
    /// `bee --connect ... jobs list`
    ListJobs,
    /// `bee --connect ... jobs inspect <id>`
    JobInspect(u32),
    /// `bee --connect ... diagnostics <id>`
    TaskDiagnostics(u32),
    /// `bee --connect ... cluster status`
    ClusterStatus,
    /// Optional ping; the test suite uses this to
    /// assert the admin RPC is wired correctly
    /// without exercising the heavier handlers.
    Ping,
    /// S33.2: list KV entries whose key starts with
    /// `prefix`. The AdminServer calls
    /// `kv.list(prefix)` under the existing KV
    /// lock; the response is a list of `(key,
    /// raw_value_bytes)` pairs. The CLI's
    /// `bee --connect <addr> kv list <prefix>`
    /// decodes per known schema or prints
    /// `(key, value_len)`.
    ListKv { prefix: String },
    /// S33.3: put a key/value into the leader's
    /// Raft KV. The AdminServer submits a
    /// `NodeCommand::Submit { op: Op::Put }` to
    /// itself (this Node); for MVP, we do not
    /// forward to the leader. The CLI's
    /// `bee --connect <addr> kv put <key>
    /// <value>` is the soak-script's tick-write
    /// path. S33.3 spec: the AdminServer is a
    /// read+write endpoint; writes are
    /// `NodeCommand::Submit`-and-await.
    KvPut { key: String, value: Vec<u8> },
    /// S33.3: deploy a SQL pipeline. The
    /// `sql_text` is the full SQL file body. The
    /// AdminServer runs the SQL through the
    /// `bee-dsl-sql::run_pipeline_with_config`
    /// path (the same path the in-process `bee
    /// run` uses). S33.3 MVP: only the leader
    /// services this request (followers reply
    /// with `AdminResponse::Error("not the
    /// leader")`).
    Deploy { sql_text: String, owner_node: u32 },
    /// S33.3: register a Datasource on the
    /// leader. Sends a `Submit { op:
    /// Op::RegisterDatasourceProducer }` so the
    /// DatasourceRegistry is updated through the
    /// Raft log (consistent across the cluster).
    /// The 4 fields mirror the existing
    /// `Datasource::new` signature.
    RegisterDatasource {
        name: String,
        adapter: String,
        plugin_version: String,
        config_json: String,
        tenant: u16,
        owner_node: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AdminResponse {
    /// `ListJobs` reply.
    JobList(Vec<JobSummary>),
    /// `JobInspect` reply.
    JobDetail(Option<JobDetail>),
    /// `TaskDiagnostics` reply.
    TaskDiag(Option<TaskDiagDetail>),
    /// `ClusterStatus` reply.
    ClusterMetrics(ClusterMetricsDetail),
    /// `Ping` reply (echoes the request id).
    Pong,
    /// Any admin RPC error (auth, parse, internal). The
    /// human-readable message is for the CLI's stderr;
    /// production should swap for a structured error
    /// type in a follow-up.
    Error(String),
    /// S33.2: reply to `ListKv`. Each tuple is
    /// `(key, bincode_body)`. The CLI decides
    /// how to render (per the optional
    /// `--raw` flag).
    KvList(Vec<(String, Vec<u8>)>),
    /// S33.3: `KvPut` reply. `ok = true` means the
    /// Raft log accepted the op; the apply loop
    /// runs asynchronously. The CLI does not wait
    /// for the apply (the soak loop is fire-and-
    /// forget for performance).
    KvPutAck { ok: bool },
    /// S33.3: `Deploy` reply. `job_id` is the
    /// newly-registered Job's id (assigned by the
    /// ControlPlane SM); 0 means the deploy
    /// failed and the error string is in
    /// `error_msg`.
    DeployAck {
        job_id: u32,
        task_ids: Vec<u32>,
        error_msg: String,
    },
    /// S33.3: `RegisterDatasource` reply.
    RegisterDatasourceAck {
        ok: bool,
        error_msg: String,
    },
}

/// Compact form of `JobRecord` for the wire. Mirrors
/// the existing `format_jobs` row shape: id, name,
/// lifecycle, mode, task count, owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSummary {
    pub job_id: u32,
    pub dag_hash: String,
    pub lifecycle: JobLifecycleState,
    /// "Producer" | "Subscriber" | "Independent"
    pub mode: String,
    pub task_count: usize,
    pub owner_node: u32,
}

impl JobSummary {
    /// Build a `JobSummary` from a `JobRecord` + the
    /// `ControlPlaneStateMachine` (needed to look up
    /// the Job's mode and task count). The plan's
    /// `From<&JobRecord>` impl needs the parent SM
    /// too because `JobRecord` does not carry `mode`
    /// directly; we resolve it from
    /// `ControlPlaneStateMachine::job_mode(id)`.
    pub fn from_record(cp: &ControlPlaneStateMachine, j: &JobRecord) -> Self {
        let mode = match cp.job_mode(j.job_id) {
            JobMode::Producer => "Producer",
            JobMode::Subscriber => "Subscriber",
            JobMode::Independent => "Independent",
        };
        let task_count = cp
            .list_tasks()
            .iter()
            .filter(|t| t.job_id == j.job_id)
            .count();
        Self {
            job_id: j.job_id,
            dag_hash: j.dag_hash.clone(),
            lifecycle: j.lifecycle,
            mode: mode.to_string(),
            task_count,
            owner_node: j.owner_node,
        }
    }
}

/// Full form of `JobRecord` for the wire. Mirrors
/// `format_job_inspect`'s output shape: id, name,
/// status, owner, deps, tasks[].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDetail {
    pub job_id: u32,
    pub dag_hash: String,
    pub lifecycle: JobLifecycleState,
    pub owner_node: u32,
    pub dependencies: Vec<JobDep>,
    pub tasks: Vec<TaskRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDep {
    pub upstream_job: u32,
    pub stream: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDiagDetail {
    pub task_id: u32,
    pub job_id: u32,
    pub phase_id: u32,
    pub status: TaskStatus,
    pub owner_node: u32,
    pub started_at_ms: u64,
    /// S33.2: per-Task runtime stats. `None` for
    /// Tasks that have not yet been observed by the
    /// Node's auto-counter (e.g. tasks on a peer
    /// node that the local Node never dispatched
    /// to). The AdminServer's `TaskDiagnostics`
    /// handler fills this from `Node::stats` at
    /// dispatch time.
    pub runtime_stats: Option<TaskRuntimeStats>,
}

impl From<&TaskRecord> for TaskDiagDetail {
    fn from(t: &TaskRecord) -> Self {
        Self {
            task_id: t.task_id,
            job_id: t.job_id,
            phase_id: t.phase_id,
            status: t.status,
            owner_node: t.owner_node,
            started_at_ms: t.started_at_ms,
            // The Node's stats map is not visible
            // here. The AdminServer's `dispatch`
            // overrides this field with the live
            // stats after the `From` conversion.
            runtime_stats: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterMetricsDetail {
    pub nodes: Vec<NodeMetricsSummary>,
    pub leader_id: Option<u32>,
    pub term: u64,
    pub commit_index: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetricsSummary {
    pub id: u32,
    pub role: String, // "Leader" | "Follower" | "Candidate"
    pub commit_index: u64,
    pub log_length: usize,
}

/// Convenience: convert an `AdminRequest` into the
/// matching `RpcMessage::Admin*` variant. The admin
/// server's RPC handler dispatches on this enum.
impl From<AdminRequest> for RpcMessage {
    fn from(req: AdminRequest) -> Self {
        match req {
            AdminRequest::ListJobs => RpcMessage::AdminListJobs,
            AdminRequest::JobInspect(id) => RpcMessage::AdminJobInspect(id),
            AdminRequest::TaskDiagnostics(id) => RpcMessage::AdminTaskDiagnostics(id),
            AdminRequest::ClusterStatus => RpcMessage::AdminClusterStatus,
            AdminRequest::Ping => RpcMessage::AdminPing,
            AdminRequest::ListKv { prefix } => {
                RpcMessage::AdminListKv(prefix)
            }
            AdminRequest::KvPut { key, value } => {
                RpcMessage::AdminKvPut { key, value }
            }
            AdminRequest::Deploy {
                sql_text,
                owner_node,
            } => RpcMessage::AdminDeploy {
                sql_text,
                owner_node,
            },
            AdminRequest::RegisterDatasource {
                name,
                adapter,
                plugin_version,
                config_json,
                tenant,
                owner_node,
            } => RpcMessage::AdminRegisterDatasource {
                name,
                adapter,
                plugin_version,
                config_json,
                tenant,
                owner_node,
            },
        }
    }
}

/// S33.2: per-Task runtime statistics, populated by
/// Node-side auto-instrumentation at the
/// `dispatch_handler` call site. The fields are
/// cumulative counters + a 1-minute rolling average
/// (computed at read time from the cumulative
/// counter, no separate sliding window needed for
/// the 5-min tick interval).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskRuntimeStats {
    /// Cumulative count of handler invocations since
    /// the Node started (or since this Task was last
    /// observed by the Node's auto-counter).
    pub messages_processed: u64,
    /// 1-min rolling average of
    /// `messages_processed`. Computed at read time
    /// (see `AdminServer::dispatch(TaskDiagnostics)`
    /// — we don't store a separate window).
    pub messages_per_sec: f64,
    /// Unix epoch ms of the last handler invocation.
    /// `0` if the Task has never been invoked.
    pub last_message_at_ms: u64,
    /// Cumulative count of handler invocations that
    /// returned `Err`.
    pub error_count: u64,
    /// Unix epoch ms of the last error. `0` if no
    /// error has been observed.
    pub last_error_at_ms: u64,
    /// The most recent error message, truncated to
    /// 1 KiB. `None` if no error has been observed.
    pub last_error: Option<String>,
}
