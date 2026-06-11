use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::control_plane::ControlPlaneStateMachine;
use crate::kv::{KVStateMachine, Op, TxnError};

use super::admin_protocol::{AdminRequest, AdminResponse, TaskRuntimeStats};
use super::transport::{InMemoryTransport, NodeTransport};
use super::types::{LogEntry, NodeCommand, NodeId, RpcMessage, Role, Term, LogIndex};

/// S33.2 type alias: a `TaskId` is the same as
/// `NodeId`/`u32` historically. We use `u32`
/// directly throughout (matching the
/// `AdminServer::dispatch` and `TaskRecord`).
pub type TaskId = u32;

/// S33.5: the type of the admin callback
/// registered on a `Node`. When the leader's
/// `Node::handle_admin_forward` decodes an
/// inner `AdminRequest`, it calls this
/// callback to dispatch to the apply path
/// (which submits the op to the local Raft
/// log).
///
/// The callback returns a boxed future
/// (async fn in trait positions isn't
/// stable, so we use a boxed future). The
/// callback's closure is `Send + Sync` so the
/// `Node` (which is `Send + Sync` via its
/// `Arc<Mutex<...>>` fields) can clone and
/// share it across tasks.
pub type AdminCallback = Arc<
    dyn Fn(AdminRequest) -> futures::future::BoxFuture<'static, AdminResponse>
        + Send
        + Sync,
>;

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

impl Default for NodeState {
    fn default() -> Self {
        Self::new()
    }
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
    // Type-erased (Arc<dyn NodeTransport>) so the
    // same `Node` runs against either `InMemoryTransport`
    // (mpsc channels; the historical path) or
    // `TcpTransport` (bee-transport sockets; the S33.1
    // multi-node path). The 4 call sites inside `Node::run`
    // call the trait's methods; `InMemoryTransport`
    // implements the trait (see transport.rs).
    transport: Arc<dyn NodeTransport>,
    state: Arc<Mutex<NodeState>>,
    kv: Arc<Mutex<KVStateMachine>>,
    cp: Arc<Mutex<ControlPlaneStateMachine>>,
    /// S33.2: per-Task runtime statistics,
    /// populated by `Node::record_task_message` /
    /// `Node::record_task_error` (called from the
    /// `dispatch_handler` call site). Read by
    /// `AdminServer::dispatch(TaskDiagnostics)`.
    stats: Arc<Mutex<HashMap<TaskId, TaskRuntimeStats>>>,
    /// S33.4: pending admin-forward replies. When
    /// a follower forwards a write to the leader,
    /// it records `(request_id, oneshot_sender)`
    /// here. The leader's reply (carried by
    /// `RpcMessage::AdminForwardReply`) is matched
    /// by `request_id` and the `Vec<u8>` response
    /// is sent to the original CLI client.
    pending_admin_replies: Arc<
        Mutex<
            HashMap<
                u64,
                tokio::sync::oneshot::Sender<Vec<u8>>,
            >,
        >,
    >,
    /// S33.4: monotonic counter for admin-forward
    /// `request_id` values. The follower's
    /// `AdminServer::dispatch` allocates a fresh
    /// id via `register_admin_reply` and embeds
    /// it in the `AdminRequest::Forward` payload.
    next_admin_request_id: Arc<std::sync::atomic::AtomicU64>,
    /// S33.5: the AdminServer's callback for
    /// handling forwarded admin writes. When
    /// the leader's `Node::handle_admin_forward`
    /// decodes the inner `AdminRequest`, it
    /// calls this callback to dispatch to the
    /// apply path (which submits the op to the
    /// local Raft log). The default is a no-op
    /// stub that returns `Error("no admin
    /// callback registered")`; `run_node`
    /// overrides it via `set_admin_callback`
    /// after constructing the Node.
    admin_callback: tokio::sync::Mutex<AdminCallback>,
    config: NodeConfig,
}

impl Node {
    pub fn new(
        self_id: NodeId,
        peer_ids: Vec<NodeId>,
        transport: Arc<dyn NodeTransport>,
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
            stats: Arc::new(Mutex::new(HashMap::new())),
            pending_admin_replies: Arc::new(Mutex::new(HashMap::new())),
            next_admin_request_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            admin_callback: tokio::sync::Mutex::new(Self::default_admin_callback()),
            config,
        }
    }

    pub fn state(&self) -> Arc<Mutex<NodeState>> {
        self.state.clone()
    }

    /// S33.2: clone the stats map handle. The
    /// `AdminServer` (Task 4) uses this to read
    /// live stats when servicing a
    /// `TaskDiagnostics` request.
    pub fn stats(&self) -> Arc<Mutex<HashMap<TaskId, TaskRuntimeStats>>> {
        self.stats.clone()
    }

    /// S33.2: record that a handler invocation
    /// for `task_id` succeeded. The caller (the
    /// `dispatch_handler` site) is responsible
    /// for trimming the 1-min rolling average
    /// (we just bump the counter; the rolling
    /// avg is computed at read time).
    pub async fn record_task_message(&self, task_id: TaskId) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut stats = self.stats.lock().await;
        let entry = stats.entry(task_id).or_default();
        entry.messages_processed = entry.messages_processed.saturating_add(1);
        entry.last_message_at_ms = now_ms;
    }

    /// S33.2: record that a handler invocation
    /// for `task_id` returned an error.
    /// `error_msg` is truncated to 1 KiB before
    /// storage.
    pub async fn record_task_error(&self, task_id: TaskId, error_msg: &str) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut stats = self.stats.lock().await;
        let entry = stats.entry(task_id).or_default();
        entry.error_count = entry.error_count.saturating_add(1);
        entry.last_error_at_ms = now_ms;
        let mut msg = error_msg.to_string();
        if msg.len() > 1024 {
            msg.truncate(1024);
        }
        entry.last_error = Some(msg);
    }

    pub fn kv(&self) -> Arc<Mutex<KVStateMachine>> {
        self.kv.clone()
    }

    pub fn cp(&self) -> Arc<Mutex<ControlPlaneStateMachine>> {
        self.cp.clone()
    }

    /// S33.4: the AdminServer uses this to push
    /// `NodeCommand::Submit { op, reply }` into
    /// the local Node's command channel. Returns
    /// the same `Arc<dyn NodeTransport>` that
    /// `Node::new` accepted.
    pub fn node_transport(&self) -> Arc<dyn NodeTransport> {
        self.transport.clone()
    }

    /// S33.4: register a pending admin-forward
    /// reply. Returns the `request_id` the
    /// follower should attach to the `Forward`
    /// payload, and a `oneshot::Receiver<Vec<u8>>`
    /// that resolves when the leader's reply
    /// arrives (via `handle_admin_forward_reply`).
    pub async fn register_admin_reply(
        &self,
    ) -> (u64, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let request_id = self
            .next_admin_request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.pending_admin_replies
            .lock()
            .await
            .insert(request_id, tx);
        (request_id, rx)
    }

    /// S33.5: the leader receives a forwarded
    /// admin write. Decode the inner
    /// `AdminRequest` and dispatch to the
    /// admin callback (which is the
    /// `AdminServer::dispatch_with_apply`
    /// machinery on the leader side).
    pub async fn handle_admin_forward(&self, to: u32, request: Vec<u8>) {
        let inner: AdminRequest = match bincode::deserialize(&request) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "handle_admin_forward: bincode decode failed: {e}"
                );
                return;
            }
        };
        // Read the (potentially updated) callback.
        let callback = {
            let guard = self.admin_callback.lock().await;
            guard.clone()
        };
        let response: AdminResponse = callback(inner).await;
        let response_bytes = match bincode::serialize(&response) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "handle_admin_forward: bincode encode failed: {e}"
                );
                return;
            }
        };
        // Send AdminForwardReply back to the
        // requester. The `request_id` is
        // hard-coded 0 for the S33.5 MVP —
        // the follower's `register_admin_reply`
        // generates a unique id per call, but
        // the leader's `handle_admin_forward`
        // doesn't see the id (it's encoded in
        // the outer `Forward` envelope, which
        // is the follower's transport-layer
        // wrapper, not the inner `AdminRequest`).
        // S33.5.1 will thread the request_id
        // through the wire so the follower can
        // match by id (not by `to`).
        if let Err(e) = self
            .transport
            .send(
                to,
                super::types::RpcMessage::AdminForwardReply {
                    to,
                    request_id: 0,
                    response: response_bytes,
                },
            )
            .await
        {
            eprintln!(
                "handle_admin_forward: send AdminForwardReply failed: {e}"
            );
        }
    }

    /// S33.4: the follower receives the leader's
    /// reply to a forwarded admin write.
    /// Match by `request_id` and send the
    /// `Vec<u8>` response to the pending
    /// `oneshot::Sender` registered by the
    /// follower's `AdminServer::dispatch`
    /// (the one waiting for the leader's reply).
    pub async fn handle_admin_forward_reply(
        &self,
        _to: u32,
        request_id: u64,
        response: Vec<u8>,
    ) {
        let mut map = self.pending_admin_replies.lock().await;
        if let Some(tx) = map.remove(&request_id) {
            let _ = tx.send(response);
        } else {
            eprintln!(
                "handle_admin_forward_reply: no pending reply for request_id={request_id}"
            );
        }
    }

    pub fn self_id(&self) -> NodeId {
        self.self_id
    }

    /// S33.5: the default `admin_callback` for
    /// Nodes that don't have a per-Node
    /// AdminServer wired up (e.g. the in-process
    /// test cluster). The default returns
    /// `Error("no admin callback registered")`
    /// for every request.
    fn default_admin_callback() -> AdminCallback {
        Arc::new(|_req: AdminRequest| {
            Box::pin(async {
                AdminResponse::Error(
                    "no admin callback registered (S33.5: run_node \
                     sets the real callback)"
                        .to_string(),
                )
            })
        })
    }

    /// S33.5: replace the admin callback. The
    /// `run_node` process calls this after
    /// constructing the Node + the AdminServer,
    /// wiring the AdminServer's
    /// `dispatch_with_apply` as the callback.
    /// Interior mutability (a `Mutex`) makes
    /// this safe to call from a `&self` (the
    /// Node is shared via `Arc`).
    pub async fn set_admin_callback<F>(&self, f: F)
    where
        F: Fn(AdminRequest) -> futures::future::BoxFuture<'static, AdminResponse>
            + Send
            + Sync
            + 'static,
    {
        let arc: AdminCallback = Arc::new(f);
        let mut guard = self.admin_callback.lock().await;
        *guard = arc;
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
                | Op::StealTask { .. }
                | Op::RegisterDatasourceProducer { .. }
                | Op::RegisterDependency { .. }
                | Op::UpdateJobLifecycle { .. } => {
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
            // S33.1 Task 8 will route the 5 Admin* variants
            // through the admin handler; until then they
            // are no-ops (the local admin server / TcpTransport
            // submit_command path are the entry points for
            // MVP; direct RpcMessage::Admin* delivery on the
            // Raft channel is not used yet).
            RpcMessage::AdminListJobs
            | RpcMessage::AdminJobInspect(_)
            | RpcMessage::AdminTaskDiagnostics(_)
            | RpcMessage::AdminClusterStatus
            | RpcMessage::AdminPing
            | RpcMessage::AdminListKv(_)
            | RpcMessage::AdminKvPut { .. }
            | RpcMessage::AdminDeploy { .. }
            | RpcMessage::AdminRegisterDatasource { .. } => {
                // No-op on the Raft channel; the
                // AdminServer on a separate port
                // is the entry point. Followers
                // forward to the leader via
                // AdminForward (Task 4).
            }
            RpcMessage::AdminForward { to, request } => {
                // Leader side: dispatch the
                // forwarded request.
                self.handle_admin_forward(to, request).await;
            }
            RpcMessage::AdminForwardReply { to, request_id, response } => {
                // Follower side: forward the
                // response to the pending
                // `oneshot` for this request_id.
                self.handle_admin_forward_reply(to, request_id, response)
                    .await;
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


/// S33.2: compute the 1-min messages/sec rate from
/// the cumulative counter. We do not have a separate
/// sliding window; instead, the AdminServer's
/// `dispatch` calls this on the live stats and
/// assumes the caller's tick interval is the
/// observation window. For the 5-min 24h-loop
/// interval, the displayed rate is the
/// `messages_processed / elapsed_minutes` (so the
/// number drifts up over the run as the cumulative
/// counter accumulates — that is acceptable for
/// 24h-soak human review; the script's threshold
/// check uses absolute `klines` and `trades` from
/// InfluxDB / MongoDB, not this rate).
pub fn messages_per_sec(messages_processed: u64, started_at_ms: u64) -> f64 {
    if started_at_ms == 0 {
        return 0.0;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let elapsed_sec = now_ms.saturating_sub(started_at_ms) as f64 / 1000.0;
    if elapsed_sec < 1.0 {
        return 0.0;
    }
    messages_processed as f64 / elapsed_sec
}
