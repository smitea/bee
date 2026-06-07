use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::control_plane::ControlPlaneStateMachine;
use crate::kv::{KVStateMachine, Op, TxnError};

use super::transport::InMemoryTransport;
use super::types::{LogEntry, NodeCommand, NodeId, RpcMessage, Role, Term, LogIndex};

#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub base_election_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub node_offset_ms: u64,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            base_election_timeout: Duration::from_millis(1000),
            heartbeat_interval: Duration::from_millis(100),
            node_offset_ms: 0,
        }
    }
}

impl NodeConfig {
    fn election_timeout(&self) -> Duration {
        self.base_election_timeout + Duration::from_millis(self.node_offset_ms)
    }
}

#[derive(Debug)]
pub struct NodeState {
    pub role: Role,
    pub current_term: Term,
    pub voted_for: Option<NodeId>,
    pub log: Vec<LogEntry>,
    pub commit_index: LogIndex,
    pub applied_index: LogIndex,
    pub last_heartbeat: Instant,
    pub leader_id: Option<NodeId>,
    pub votes_received: u32,
    pub next_index: HashMap<NodeId, LogIndex>,
    pub match_index: HashMap<NodeId, LogIndex>,
}

impl NodeState {
    fn new() -> Self {
        Self {
            role: Role::Follower,
            current_term: 0,
            voted_for: None,
            log: Vec::new(),
            commit_index: 0,
            applied_index: 0,
            last_heartbeat: Instant::now(),
            leader_id: None,
            votes_received: 0,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
        }
    }

    fn last_log_index(&self) -> LogIndex {
        self.log.len() as LogIndex
    }

    fn last_log_term(&self) -> Term {
        self.log.last().map(|e| e.term).unwrap_or(0)
    }

    fn is_log_up_to_date(&self, last_index: LogIndex, last_term: Term) -> bool {
        let my_last_term = self.last_log_term();
        let my_last_index = self.last_log_index();
        last_term > my_last_term
            || (last_term == my_last_term && last_index >= my_last_index)
    }
}

pub struct Node {
    self_id: NodeId,
    peer_ids: Vec<NodeId>,
    transport: InMemoryTransport,
    state: Arc<Mutex<NodeState>>,
    kv: Arc<Mutex<KVStateMachine>>,
    cp: Arc<Mutex<ControlPlaneStateMachine>>,
    config: NodeConfig,
}

impl Node {
    pub fn new(
        self_id: NodeId,
        peer_ids: Vec<NodeId>,
        transport: InMemoryTransport,
        kv: Arc<Mutex<KVStateMachine>>,
        cp: Arc<Mutex<ControlPlaneStateMachine>>,
        config: NodeConfig,
    ) -> Self {
        Self {
            self_id,
            peer_ids,
            transport,
            state: Arc::new(Mutex::new(NodeState::new())),
            kv,
            cp,
            config,
        }
    }

    pub fn state(&self) -> Arc<Mutex<NodeState>> {
        self.state.clone()
    }

    pub fn kv(&self) -> Arc<Mutex<KVStateMachine>> {
        self.kv.clone()
    }

    pub fn cp(&self) -> Arc<Mutex<ControlPlaneStateMachine>> {
        self.cp.clone()
    }

    pub fn self_id(&self) -> NodeId {
        self.self_id
    }

    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    pub async fn run(self) {
        let election_timeout = self.config.election_timeout();
        let mut next_election = Instant::now() + election_timeout;
        let mut next_heartbeat = Instant::now() + Duration::from_secs(3600);

        loop {
            let now = Instant::now();
            let sleep_dur = next_election.min(next_heartbeat).saturating_duration_since(now);

            tokio::select! {
                biased;
                cmd = self.transport.recv_cmd() => match cmd {
                    Some(NodeCommand::Submit { op, reply }) => {
                        let result = self.handle_submit(op).await;
                        let _ = reply.send(result);
                    }
                    Some(NodeCommand::Shutdown) | None => break,
                },
                rpc = self.transport.recv_rpc() => if let Some((_from, msg)) = rpc {
                    self.handle_rpc(msg).await;
                    let mut state = self.state.lock().await;
                    state.last_heartbeat = Instant::now();
                    if state.role == Role::Follower {
                        next_election = Instant::now() + election_timeout;
                    }
                },
                _ = tokio::time::sleep(sleep_dur) => {
                    let now = Instant::now();
                    if now >= next_election {
                        self.check_election_timeout().await;
                        next_election = now + election_timeout;
                    }
                    if now >= next_heartbeat {
                        if self.is_leader().await {
                            self.broadcast_heartbeat().await;
                        }
                        next_heartbeat = now + self.config.heartbeat_interval;
                    }
                }
            }
        }
    }

    async fn is_leader(&self) -> bool {
        self.state.lock().await.role == Role::Leader
    }

    async fn handle_submit(&self, op: Op) -> Result<(), TxnError> {
        let (is_leader, current_term) = {
            let state = self.state.lock().await;
            (state.role == Role::Leader, state.current_term)
        };
        if !is_leader {
            return Err(TxnError::Conflict {
                key: format!("not_leader_node_{}", self.self_id),
                expected: None,
                actual: Some(format!("term_{current_term}").into_bytes()),
            });
        }
        let entry = LogEntry::new(current_term, op);
        let entry_index;
        {
            let mut state = self.state.lock().await;
            state.log.push(entry);
            entry_index = state.log.len() as LogIndex;
            state.match_index.insert(self.self_id, entry_index);
        }
        self.try_advance_commit_index().await;
        self.broadcast_append_entries().await;
        Ok(())
    }

    async fn try_advance_commit_index(&self) {
        let new_commit = {
            let mut state = self.state.lock().await;
            if state.role != Role::Leader {
                return;
            }
            let mut indices: Vec<LogIndex> = state.match_index.values().copied().collect();
            indices.push(state.log.len() as LogIndex);
            indices.sort_unstable();
            let majority_index = indices[indices.len() / 2];
            if majority_index > state.commit_index
                && state.log.get((majority_index - 1) as usize).map(|e| e.term)
                    == Some(state.current_term)
            {
                state.commit_index = majority_index;
                Some(majority_index)
            } else {
                None
            }
        };
        if let Some(c) = new_commit {
            self.apply_committed(c).await;
        }
    }

    async fn apply_committed(&self, up_to: LogIndex) {
        let to_apply: Vec<LogEntry>;
        {
            let mut state = self.state.lock().await;
            if up_to <= state.applied_index {
                return;
            }
            let start = state.applied_index as usize;
            let end = (up_to as usize).min(state.log.len());
            if end <= start {
                return;
            }
            to_apply = state.log[start..end].to_vec();
            state.applied_index = up_to;
        }
        let mut kv = self.kv.lock().await;
        let mut cp = self.cp.lock().await;
        for entry in to_apply {
            match &entry.op {
                Op::Put { .. } | Op::Del { .. } | Op::Cas { .. } | Op::Txn { .. } => {
                    let _ = kv.apply_op(&entry.op);
                }
                Op::RegisterJob { .. }
                | Op::RegisterTask { .. }
                | Op::UpdateTaskStatus { .. }
                | Op::Heartbeat { .. }
                | Op::StealTask { .. } => {
                    let _ = cp.apply_op(&entry.op);
                }
            }
        }
    }

    async fn handle_rpc(&self, msg: RpcMessage) {
        match msg {
            RpcMessage::RequestVote {
                term,
                candidate_id,
                last_log_index,
                last_log_term,
            } => {
                self.handle_request_vote(term, candidate_id, last_log_index, last_log_term)
                    .await;
            }
            RpcMessage::RequestVoteReply {
                term,
                voter_id: _,
                vote_granted,
            } => {
                self.handle_request_vote_reply(term, vote_granted).await;
            }
            RpcMessage::AppendEntries {
                term,
                leader_id,
                prev_log_index,
                prev_log_term,
                entries,
                leader_commit,
            } => {
                self.handle_append_entries(
                    term,
                    leader_id,
                    prev_log_index,
                    prev_log_term,
                    entries,
                    leader_commit,
                )
                .await;
            }
            RpcMessage::AppendEntriesReply {
                term,
                follower_id,
                success,
                match_index,
            } => {
                self.handle_append_entries_reply(follower_id, term, success, match_index)
                    .await;
            }
            RpcMessage::Heartbeat { term, leader_id } => {
                self.handle_heartbeat(term, leader_id).await;
            }
            RpcMessage::HeartbeatReply { term, follower_id: _ } => {
                self.handle_heartbeat_reply(term).await;
            }
        }
    }

    async fn handle_request_vote(
        &self,
        term: Term,
        candidate_id: NodeId,
        last_log_index: LogIndex,
        last_log_term: Term,
    ) {
        let (vote_granted, current_term) = {
            let mut state = self.state.lock().await;
            if term > state.current_term {
                state.current_term = term;
                state.role = Role::Follower;
                state.voted_for = None;
                state.leader_id = None;
            }
            if term < state.current_term {
                (false, state.current_term)
            } else {
                let log_ok = state.is_log_up_to_date(last_log_index, last_log_term);
                let already_voted =
                    state.voted_for.is_some() && state.voted_for != Some(candidate_id);
                if log_ok && !already_voted {
                    state.voted_for = Some(candidate_id);
                    (true, state.current_term)
                } else {
                    (false, state.current_term)
                }
            }
        };
        let _ = self
            .transport
            .send(
                candidate_id,
                RpcMessage::RequestVoteReply {
                    term: current_term,
                    voter_id: self.self_id,
                    vote_granted,
                },
            )
            .await;
    }

    async fn handle_request_vote_reply(&self, term: Term, vote_granted: bool) {
        let mut state = self.state.lock().await;
        if term > state.current_term {
            state.current_term = term;
            state.role = Role::Follower;
            state.voted_for = None;
            state.leader_id = None;
            return;
        }
        if state.role != Role::Candidate || term != state.current_term {
            return;
        }
        if vote_granted {
            state.votes_received += 1;
            let total = self.peer_ids.len() as u32 + 1;
            if state.votes_received * 2 > total {
                let old_role = state.role;
                state.role = Role::Leader;
                state.leader_id = Some(self.self_id);
                let last_index = state.log.len() as LogIndex;
                for &peer in &self.peer_ids {
                    state.next_index.insert(peer, last_index + 1);
                    state.match_index.insert(peer, 0);
                }
                if old_role != Role::Leader {
                    drop(state);
                    self.broadcast_heartbeat().await;
                }
            }
        }
    }

    async fn handle_heartbeat(&self, term: Term, leader_id: NodeId) {
        let mut state = self.state.lock().await;
        if term > state.current_term {
            state.current_term = term;
            state.role = Role::Follower;
            state.voted_for = None;
        }
        if term >= state.current_term {
            state.leader_id = Some(leader_id);
        }
    }

    async fn handle_heartbeat_reply(&self, _term: Term) {}

    async fn handle_append_entries(
        &self,
        term: Term,
        leader_id: NodeId,
        prev_log_index: LogIndex,
        prev_log_term: Term,
        entries: Vec<LogEntry>,
        leader_commit: LogIndex,
    ) {
        let (success, current_term, match_index) = {
            let mut state = self.state.lock().await;
            if term > state.current_term {
                state.current_term = term;
                state.role = Role::Follower;
                state.voted_for = None;
            }
            if term < state.current_term {
                (false, state.current_term, 0)
            } else {
                state.leader_id = Some(leader_id);
                let prev_ok = if prev_log_index == 0 {
                    true
                } else {
                    state
                        .log
                        .get((prev_log_index - 1) as usize)
                        .map(|e| e.term == prev_log_term)
                        .unwrap_or(false)
                };
                if !prev_ok {
                    (false, state.current_term, state.log.len() as LogIndex)
                } else {
                    let start = prev_log_index as usize;
                    for (i, entry) in entries.into_iter().enumerate() {
                        let idx = start + i;
                        if let Some(existing) = state.log.get(idx) {
                            if existing.term != entry.term {
                                state.log.truncate(idx);
                                state.log.push(entry);
                            }
                        } else {
                            state.log.push(entry);
                        }
                    }
                    if leader_commit > state.commit_index {
                        let last_new = state.log.len() as LogIndex;
                        state.commit_index = leader_commit.min(last_new);
                    }
                    (
                        true,
                        state.current_term,
                        state.log.len() as LogIndex,
                    )
                }
            }
        };
        let _ = self
            .transport
            .send(
                leader_id,
                RpcMessage::AppendEntriesReply {
                    term: current_term,
                    follower_id: self.self_id,
                    success,
                    match_index,
                },
            )
            .await;
        if success {
            self.apply_committed_current().await;
        }
    }

    async fn apply_committed_current(&self) {
        let commit = self.state.lock().await.commit_index;
        self.apply_committed(commit).await;
    }

    async fn handle_append_entries_reply(
        &self,
        follower_id: NodeId,
        term: Term,
        success: bool,
        match_index: LogIndex,
    ) {
        let should_advance = {
            let mut state = self.state.lock().await;
            if term > state.current_term {
                state.current_term = term;
                state.role = Role::Follower;
                state.voted_for = None;
                state.leader_id = None;
                return;
            }
            if state.role != Role::Leader || term != state.current_term {
                return;
            }
            if success {
                state.match_index.insert(follower_id, match_index);
                state.next_index.insert(follower_id, match_index + 1);
                true
            } else {
                false
            }
        };
        if should_advance {
            let old_commit = self.state.lock().await.commit_index;
            self.try_advance_commit_index().await;
            let new_commit = self.state.lock().await.commit_index;
            if new_commit > old_commit {
                self.broadcast_append_entries().await;
            }
        }
    }

    async fn check_election_timeout(&self) {
        let should_start = {
            let state = self.state.lock().await;
            if state.role == Role::Leader {
                false
            } else {
                let elapsed = state.last_heartbeat.elapsed();
                elapsed >= self.config.election_timeout()
            }
        };
        if should_start {
            self.start_election().await;
        }
    }

    async fn start_election(&self) {
        let (term, last_index, last_term) = {
            let mut state = self.state.lock().await;
            state.current_term += 1;
            state.role = Role::Candidate;
            state.voted_for = Some(self.self_id);
            state.votes_received = 1;
            state.leader_id = None;
            (
                state.current_term,
                state.log.len() as LogIndex,
                state.log.last().map(|e| e.term).unwrap_or(0),
            )
        };
        for &peer in &self.peer_ids {
            let _ = self
                .transport
                .send(
                    peer,
                    RpcMessage::RequestVote {
                        term,
                        candidate_id: self.self_id,
                        last_log_index: last_index,
                        last_log_term: last_term,
                    },
                )
                .await;
        }
    }

    async fn broadcast_heartbeat(&self) {
        let (term, leader_id) = {
            let state = self.state.lock().await;
            (state.current_term, self.self_id)
        };
        for &peer in &self.peer_ids {
            let _ = self
                .transport
                .send(peer, RpcMessage::Heartbeat { term, leader_id })
                .await;
        }
    }

    async fn broadcast_append_entries(&self) {
        let (term, leader_id, log_snapshot) = {
            let state = self.state.lock().await;
            if state.role != Role::Leader {
                return;
            }
            (state.current_term, self.self_id, state.log.clone())
        };
        for &peer in &self.peer_ids {
            let next_index = {
                let state = self.state.lock().await;
                state.next_index.get(&peer).copied().unwrap_or(1)
            };
            let prev_log_index = if next_index == 0 { 0 } else { next_index - 1 };
            let prev_log_term = if prev_log_index == 0 {
                0
            } else {
                log_snapshot
                    .get((prev_log_index - 1) as usize)
                    .map(|e| e.term)
                    .unwrap_or(0)
            };
            let entries: Vec<LogEntry> = log_snapshot
                .iter()
                .skip(prev_log_index as usize)
                .cloned()
                .collect();
            let commit_index = self.state.lock().await.commit_index;
            let _ = self
                .transport
                .send(
                    peer,
                    RpcMessage::AppendEntries {
                        term,
                        leader_id,
                        prev_log_index,
                        prev_log_term,
                        entries,
                        leader_commit: commit_index,
                    },
                )
                .await;
        }
    }
}
