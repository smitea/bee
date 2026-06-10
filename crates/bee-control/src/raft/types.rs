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
}

#[derive(Debug)]
pub enum NodeCommand {
    Submit {
        op: Op,
        reply: tokio::sync::oneshot::Sender<Result<(), crate::kv::TxnError>>,
    },
    Shutdown,
}
