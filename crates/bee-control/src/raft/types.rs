use serde::{Deserialize, Serialize};

use crate::kv::Op;

pub type NodeId = u32;
pub type Term = u64;
pub type LogIndex = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Follower,
    Candidate,
    Leader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub term: Term,
    pub op: Op,
}

impl LogEntry {
    pub fn new(term: Term, op: Op) -> Self {
        Self { term, op }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RpcMessage {
    RequestVote {
        term: Term,
        candidate_id: NodeId,
        last_log_index: LogIndex,
        last_log_term: Term,
    },
    RequestVoteReply {
        term: Term,
        voter_id: NodeId,
        vote_granted: bool,
    },
    AppendEntries {
        term: Term,
        leader_id: NodeId,
        prev_log_index: LogIndex,
        prev_log_term: Term,
        entries: Vec<LogEntry>,
        leader_commit: LogIndex,
    },
    AppendEntriesReply {
        term: Term,
        follower_id: NodeId,
        success: bool,
        match_index: LogIndex,
    },
    Heartbeat {
        term: Term,
        leader_id: NodeId,
    },
    HeartbeatReply {
        term: Term,
        follower_id: NodeId,
    },
    /// S33.1: admin RPC request from a remote `bee --connect`
    /// client. The receiving Node handles it locally (or
    /// forwards to the leader if the request is a write).
    /// MVP: all admin requests are reads; every Node can
    /// serve them from its own state machine.
    AdminListJobs,
    AdminJobInspect(u32),
    AdminTaskDiagnostics(u32),
    AdminClusterStatus,
    AdminPing,
    /// S33.2: list KV entries by prefix.
    AdminListKv(String),
    /// S33.3: put a key/value (soak-script tick write).
    AdminKvPut { key: String, value: Vec<u8> },
    /// S33.3: deploy a SQL pipeline (the soak
    /// script's Phase 4, gated on the leader).
    AdminDeploy {
        sql_text: String,
        owner_node: u32,
    },
    /// S33.3: register a Datasource (the soak
    /// script's Phase 3, gated on the leader).
    AdminRegisterDatasource {
        name: String,
        adapter: String,
        plugin_version: String,
        config_json: String,
        tenant: u16,
        owner_node: u32,
    },
    /// S33.4: follower -> leader admin write
    /// forward. `request` is
    /// bincode(AdminRequest).
    AdminForward { to: u32, request: Vec<u8>, request_id: u64 },
    /// S33.4: leader -> follower admin write
    /// reply. The follower's `Node::handle_rpc`
    /// matches the `request_id` and forwards
    /// the `response` to a pending `oneshot`
    /// sender.
    AdminForwardReply { to: u32, request_id: u64, response: Vec<u8> },
}

#[derive(Debug)]
pub enum NodeCommand {
    Submit {
        op: Op,
        reply: tokio::sync::oneshot::Sender<Result<(), crate::kv::TxnError>>,
    },
    Shutdown,
}
