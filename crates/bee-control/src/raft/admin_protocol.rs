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
        }
    }
}
