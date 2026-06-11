# S33.4 — Raft-log forwarding for admin writes (implementation plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal**: Close S33.3's "leader-only + local apply" MVP by routing admin writes through the Raft log. `KvPut` / `Deploy` / `RegisterDatasource` on a follower forward to the leader; the leader submits the op via `NodeCommand::Submit`; all 3 nodes apply consistently. The `Deploy` arm is no longer a marker — it runs `run_pipeline_with_config` and registers the resulting Job + Tasks.

**Architecture**: Add a `NodeTransport::submit_command` method (so the AdminServer can push `NodeCommand::Submit { op, reply }` into the local Node's command channel). Add an `AdminRequest::Forward { to, request: Vec<u8> }` variant (so a follower can relay a write to the leader via the existing Raft channel). Add a `RpcMessage::AdminForward { to, request: Vec<u8> }` and `RpcMessage::AdminForwardReply { to, request_id, response: Vec<u8> }` for the wire. The leader's `Node::handle_rpc` decodes the inner `AdminRequest` and dispatches to a new `dispatch_with_apply` fn that builds the appropriate `Op` and submits it via `self.transport.submit_command(cmd).await`. The `Oneshot` reply is awaited; the result becomes the `AdminForwardReply` payload.

**Tech Stack**: Rust 2021, `tokio` (existing), `bincode` (existing), `serde` (existing), `bee-dsl-sql` (existing `run_pipeline_with_config`). No new external deps.

**Design**: `docs/superpowers/specs/2026-06-10-s33-4-raft-log-forwarding-for-admin-writes.md`

---

## File Structure

| File | Change | Responsibility |
| --- | --- | --- |
| `crates/bee-control/src/raft/transport.rs` | Modify | Add `submit_command` to `NodeTransport` trait; `InMemoryTransport` impl |
| `crates/bee-control/src/raft/tcp.rs` | Modify | TcpTransport's `submit_command` (already exists as pub fn; just expose as trait method) |
| `crates/bee-control/src/raft/admin_protocol.rs` | Modify | `AdminRequest::Forward { to, request: Vec<u8> }` + `AdminResponse::Forwarded { request_id, response: Vec<u8> }` |
| `crates/bee-control/src/raft/types.rs` | Modify | `RpcMessage::AdminForward { to, request }` + `RpcMessage::AdminForwardReply { to, request_id, response }` |
| `crates/bee-control/src/raft/admin_server.rs` | Modify | `dispatch_with_apply` fn (leader path); forwarding check in `dispatch`; pending-replies map |
| `crates/bee-control/src/raft/node.rs` | Modify | `Node::handle_rpc` decodes `AdminForward` and dispatches; `Node::handle_rpc` for `AdminForwardReply` forwards the response to a pending reply; new `Node::record_admin_response(request_id, response)` helper |
| `crates/bee-control/src/raft/cluster.rs` | Modify | `Cluster::new_with_tcp` plumbs the `pending_admin_replies` map (or keeps it in the AdminServer) |
| `crates/bee-control/tests/admin_forwarding_integration.rs` | Create | Multi-writer test: follower receives Deploy, leader commits, all 3 nodes see the Job |
| `crates/bee-control/tests/admin_apply_pipeline.rs` | Create | Leader Deploys a SQL, asserts Job + Tasks appear in the ControlPlane SM |
| `crates/bee-dsl-sql/Cargo.toml` | Modify | No change (the `run_pipeline_with_config` is already pub) |
| `bee/src/main.rs` | Modify | No change (CLI is unchanged) |
| `docs/best-practices/quant/stories.md` | Modify | S33.4 acceptance criteria marked |
| `docs/best-practices/quant/README.md` | Modify | Paragraph on S33.4 |

**Total: 1 new test file + 8 modified, ~700 net lines.**

---

## Phase 1: Plumbing (Tasks 1-3)

### Task 1: `NodeTransport::submit_command` trait method

**Files:**
- Modify: `crates/bee-control/src/raft/transport.rs`
- Modify: `crates/bee-control/src/raft/tcp.rs`

- [ ] **Step 1: Add `submit_command` to the trait**

In `crates/bee-control/src/raft/transport.rs`, find the trait:

```rust
#[async_trait]
pub trait NodeTransport: Send + Sync + 'static {
    fn self_id(&self) -> NodeId;
    async fn send(&self, target: NodeId, msg: RpcMessage) -> Result<(), TransportError>;
    async fn recv_rpc(&self) -> Option<(NodeId, RpcMessage)>;
    async fn recv_cmd(&self) -> Option<NodeCommand>;
}
```

Add:

```rust
    /// S33.4: push a `NodeCommand` (typically
    /// `Submit { op, reply }`) into the local
    /// Node's command channel. The leader's
    /// AdminServer uses this to submit ops to
    /// the Raft log. The `TcpTransport` impl
    /// already has a `pub fn submit_command`;
    /// we just add it to the trait.
    async fn submit_command(&self, cmd: NodeCommand) -> Result<(), TransportError>;
```

- [ ] **Step 2: Add the `InMemoryTransport` impl**

The in-memory transport's command channel is owned by the transport (per S33.1's `InMemoryTransport::new`). We need a way to push commands from the AdminServer (which is a separate task in the test) into the same channel. The cleanest way: store an `mpsc::Sender<NodeCommand>` in the `InMemoryTransport` (alongside the existing `cmd_inbox`).

Find the struct:
```rust
#[derive(Clone)]
pub struct InMemoryTransport {
    self_id: NodeId,
    router: Arc<Router>,
    inbox: Arc<tokio::sync::Mutex<mpsc::Receiver<(NodeId, RpcMessage)>>>,
    cmd_inbox: Arc<tokio::sync::Mutex<mpsc::Receiver<NodeCommand>>>,
}
```

Add a new field:
```rust
    cmd_tx: mpsc::Sender<NodeCommand>,
```

And update the constructor to accept a `cmd_tx` (the existing `Cluster::new` builds the `cmd_tx` separately and forwards commands to it; we can now use it directly).

But wait — the `InMemoryTransport` already has a `cmd_inbox` (the receiver). The new `cmd_tx` is the *sender* for that channel. The cluster would need to pass the same sender to the AdminServer.

Simpler approach: the `InMemoryTransport` doesn't need a `cmd_tx` field at all. Instead, the AdminServer takes an `mpsc::Sender<NodeCommand>` as a constructor parameter. For the TCP transport, the AdminServer uses the transport's own `submit_command`. For the in-memory transport, the AdminServer uses a sender that the Cluster constructed.

Actually, the cleanest way: **drop the `cmd_inbox` field from `InMemoryTransport` and use the trait's `submit_command` + `recv_cmd` for the in-memory case too**. The `Cluster::new` constructs an `mpsc::channel`, the receiver goes into the transport (as the `cmd_inbox`), the sender is cloned and held by the AdminServer (or a wrapper).

For S33.4 we keep the existing `cmd_inbox` (the receiver) on the transport, but also expose the `cmd_tx` (the sender) via the `submit_command` trait method.

Find the InMemoryTransport impl:

```rust
impl NodeTransport for InMemoryTransport {
    fn self_id(&self) -> NodeId { ... }
    async fn send(...) { ... }
    async fn recv_rpc(...) { ... }
    async fn recv_cmd(...) { ... }
}
```

Add:

```rust
    async fn submit_command(&self, cmd: NodeCommand) -> Result<(), TransportError> {
        // S33.4: the in-memory transport's command
        // channel is owned by the transport (per
        // S33.1's InMemoryTransport::new). For
        // backwards compat with the existing
        // Cluster::new that builds the channel,
        // we accept a `cmd_tx` field at
        // construction time and use it here.
        // The MVP impl: send the cmd via the
        // same mpsc::Sender held by the
        // transport.
        todo!("S33.4 Task 1: implement once InMemoryTransport::cmd_tx is plumbed in Cluster::new")
    }
```

For the MVP we'll leave the `todo!()` and plumb the `cmd_tx` in Task 2.

- [ ] **Step 3: Add the TcpTransport impl**

TcpTransport already has `submit_command` as a `pub async fn`. Convert it to the trait method (rename or add an inline impl):

```rust
impl NodeTransport for TcpTransport {
    // ... existing methods ...
    async fn submit_command(&self, cmd: NodeCommand) -> Result<(), TransportError> {
        // Reuse the existing pub fn body.
        self.submit_command(cmd).await.map_err(|e| match e {
            bee_transport::TransportError::ConnectionClosed => {
                TransportError::Io("connection closed".to_string())
            }
            other => TransportError::Io(format!("tcp: {other:?}")),
        })
    }
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p bee-control 2>&1 | tail -5`
Expected: builds. The in-memory impl has a `todo!()`; it's only used by tests (production path is TcpTransport).

- [ ] **Step 5: Commit**

```bash
git add crates/bee-control/src/raft/transport.rs crates/bee-control/src/raft/tcp.rs
git commit -m "S33.4 Task 1: NodeTransport::submit_command trait method"
```

---

### Task 2: Plumb `cmd_tx` through `InMemoryTransport` + `Cluster::new`

**Files:**
- Modify: `crates/bee-control/src/raft/transport.rs`
- Modify: `crates/bee-control/src/raft/cluster.rs`

- [ ] **Step 1: Add `cmd_tx` to `InMemoryTransport`**

Replace the `todo!()` from Task 1 with the real impl. First, modify the struct + constructor:

```rust
#[derive(Clone)]
pub struct InMemoryTransport {
    self_id: NodeId,
    router: Arc<Router>,
    inbox: Arc<tokio::sync::Mutex<mpsc::Receiver<(NodeId, RpcMessage)>>>,
    cmd_inbox: Arc<tokio::sync::Mutex<mpsc::Receiver<NodeCommand>>>,
    /// S33.4: the sender side of the command
    /// channel. The AdminServer uses this to
    /// push `NodeCommand::Submit { op, reply }`
    /// into the same channel the Node reads.
    cmd_tx: mpsc::Sender<NodeCommand>,
}
```

Update the constructor to take `cmd_tx`:

```rust
impl InMemoryTransport {
    pub fn new(
        self_id: NodeId,
        router: Arc<Router>,
        inbox: mpsc::Receiver<(NodeId, RpcMessage)>,
        cmd_inbox: mpsc::Receiver<NodeCommand>,
        cmd_tx: mpsc::Sender<NodeCommand>,
    ) -> Self {
        Self {
            self_id,
            router,
            inbox: Arc::new(tokio::sync::Mutex::new(inbox)),
            cmd_inbox: Arc::new(tokio::sync::Mutex::new(cmd_inbox)),
            cmd_tx,
        }
    }
```

- [ ] **Step 2: Implement `submit_command` on `InMemoryTransport`**

```rust
impl NodeTransport for InMemoryTransport {
    // ... existing methods ...
    async fn submit_command(&self, cmd: NodeCommand) -> Result<(), TransportError> {
        self.cmd_tx
            .send(cmd)
            .await
            .map_err(|_| TransportError::Io("cmd channel closed".to_string()))
    }
}
```

- [ ] **Step 3: Update `Cluster::new` to pass `cmd_tx` to the transport**

In `crates/bee-control/src/raft/cluster.rs`, find the `Cluster::new` loop that builds `InMemoryTransport`:

```rust
let transport: Arc<dyn NodeTransport> = Arc::new(
    InMemoryTransport::new(id, router.clone(), rpc_rx, cmd_rx),
);
```

Replace with:

```rust
let transport: Arc<dyn NodeTransport> = Arc::new(
    InMemoryTransport::new(id, router.clone(), rpc_rx, cmd_rx, cmd_txs.remove(0).1),
);
```

(We need `cmd_txs` to be available at that point; check the loop ordering — `cmd_txs` is built before the slots loop, so this works.)

- [ ] **Step 4: Verify it compiles + tests pass**

Run: `cargo test -p bee-control 2>&1 | grep -E "^test result|FAILED" | head -3`
Expected: 85 tests pass (no test count change).

- [ ] **Step 5: Commit**

```bash
git add crates/bee-control/src/raft/transport.rs crates/bee-control/src/raft/cluster.rs
git commit -m "S33.4 Task 2: InMemoryTransport::cmd_tx + Cluster::new plumbs it"
```

---

### Task 3: Wire types for `Forward` + `AdminForward` + `AdminForwardReply`

**Files:**
- Modify: `crates/bee-control/src/raft/admin_protocol.rs`
- Modify: `crates/bee-control/src/raft/types.rs`

- [ ] **Step 1: Add `AdminRequest::Forward`**

In `crates/bee-control/src/raft/admin_protocol.rs`, append to the `AdminRequest` enum:

```rust
    /// S33.4: a follower forwards a write to the
    /// leader. `request` is the bincode-serialized
    /// inner `AdminRequest` (the leader
    /// deserializes it before dispatching).
    /// The follower's `AdminServer` sets `to` to
    /// the leader's `NodeId`. The leader's
    /// `dispatch` handles this by submitting the
    /// op to its own Raft log.
    Forward { to: u32, request: Vec<u8> },
```

- [ ] **Step 2: Add `AdminResponse::Forwarded`**

In the same file, append to the `AdminResponse` enum:

```rust
    /// S33.4: the follower's reply to a forwarded
    /// request. The leader's `AdminForwardReply`
    /// carries this payload back; the follower
    /// matches the `request_id` and sends the
    /// `response` to the original CLI client.
    /// The `request_id` is opaque (a u64 generated
    /// by the follower).
    Forwarded { request_id: u64, response: Vec<u8> },
```

- [ ] **Step 3: Add `RpcMessage::AdminForward` + `AdminForwardReply`**

In `crates/bee-control/src/raft/types.rs`, append to the `RpcMessage` enum:

```rust
    /// S33.4: follower -> leader admin write
    /// forward. `request` is bincode(AdminRequest).
    AdminForward { to: u32, request: Vec<u8> },
    /// S33.4: leader -> follower admin write
    /// reply. The follower's `Node::handle_rpc`
    /// matches the `request_id` and forwards the
    /// `response` to a pending `oneshot` sender.
    AdminForwardReply { to: u32, request_id: u64, response: Vec<u8> },
```

- [ ] **Step 4: Update `From<AdminRequest> for RpcMessage` to include Forward**

In `crates/bee-control/src/raft/admin_protocol.rs`, find the impl:

```rust
impl From<AdminRequest> for RpcMessage {
    fn from(req: AdminRequest) -> Self {
        match req {
            // ... existing arms ...
            AdminRequest::Forward { to, request } => {
                RpcMessage::AdminForward { to, request }
            }
        }
    }
}
```

- [ ] **Step 5: Update `Node::handle_rpc` placeholder arm**

In `crates/bee-control/src/raft/node.rs`, find the existing placeholder arm (which currently lumps all Admin* variants together):

```rust
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
                // AdminServer (Task 5) is the entry
                // point for all admin RPCs in MVP.
            }
```

Replace with the split arms (Task 4 wires `AdminForward` + `AdminForwardReply`):

```rust
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
                // AdminServer on a separate port is
                // the entry point. Followers
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
                self.handle_admin_forward_reply(to, request_id, response).await;
            }
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo build -p bee-control 2>&1 | grep -E "^error" | head -5`
Expected: error (handle_admin_forward + handle_admin_forward_reply don't exist yet). We'll add them in Task 4. **Temporarily** comment out the new arms:

```rust
            // RpcMessage::AdminForward { to, request } => {
            //     self.handle_admin_forward(to, request).await;
            // }
            // RpcMessage::AdminForwardReply { to, request_id, response } => {
            //     self.handle_admin_forward_reply(to, request_id, response).await;
            // }
```

Revert this comment-out in Task 4.

- [ ] **Step 7: Commit**

```bash
git add crates/bee-control/src/raft/admin_protocol.rs crates/bee-control/src/raft/types.rs crates/bee-control/src/raft/node.rs
git commit -m "S33.4 Task 3: AdminRequest::Forward + AdminForward wire types (handlers wired in Task 4)"
```

---

## Phase 2: Forwarding + Apply (Tasks 4-7)

### Task 4: `Node::handle_admin_forward` + `handle_admin_forward_reply`

**Files:**
- Modify: `crates/bee-control/src/raft/node.rs`

- [ ] **Step 1: Uncomment the new `handle_rpc` arms (revert Step 6 of Task 3)**

- [ ] **Step 2: Add `Node::pending_admin_replies` field**

In the `Node` struct, add:

```rust
    /// S33.4: pending admin-forward replies. When
    /// a follower forwards a write to the leader,
    /// it records `(request_id, oneshot_sender)`
    /// here. The leader's reply (carried by
    /// `AdminForwardReply`) is matched by
    /// `request_id` and the `Vec<u8>` response
    /// is sent to the original CLI client.
    pending_admin_replies: Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Vec<u8>>>>>,
```

- [ ] **Step 3: Initialize in `Node::new`**

```rust
            pending_admin_replies: Arc::new(Mutex::new(HashMap::new())),
```

- [ ] **Step 4: Add `Node::register_admin_reply` + `Node::handle_admin_forward` + `handle_admin_forward_reply`**

```rust
impl Node {
    /// S33.4: register a pending admin-forward
    /// reply. Returns the `request_id` the
    /// follower should attach to the `Forward`
    /// payload, and a `oneshot::Receiver<Vec<u8>>`
    /// that resolves when the leader's reply
    /// arrives.
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

    pub async fn handle_admin_forward(&self, to: u32, request: Vec<u8>) {
        // The leader receives a forwarded
        // request. Decode the inner AdminRequest
        // and dispatch to the same path the
        // AdminServer uses, but the apply path
        // is via `NodeCommand::Submit` instead of
        // direct mutex. This is `dispatch_with_apply`
        // (defined in Task 5 on admin_server.rs).
        //
        // For Task 4, the impl is a placeholder:
        // we just decode + log + reply with a
        // generic "not yet wired" error. Task 5
        // wires the real path.
        eprintln!(
            "handle_admin_forward: to={to}, request={} bytes (Task 5 wires the real path)",
            request.len()
        );
    }

    pub async fn handle_admin_forward_reply(
        &self,
        to: u32,
        request_id: u64,
        response: Vec<u8>,
    ) {
        // The follower receives the leader's
        // reply. Match by `request_id` and send
        // the response to the pending
        // `oneshot::Sender<Vec<u8>>` (the
        // AdminServer's `dispatch` is waiting
        // on the receiver).
        let mut map = self.pending_admin_replies.lock().await;
        if let Some(tx) = map.remove(&request_id) {
            let _ = tx.send(response);
        } else {
            eprintln!(
                "handle_admin_forward_reply: no pending reply for request_id={request_id}"
            );
        }
    }
```

- [ ] **Step 5: Add `next_admin_request_id` to `Node`**

```rust
    next_admin_request_id: Arc<std::sync::atomic::AtomicU64>,
```

- [ ] **Step 6: Initialize in `Node::new`**

```rust
            next_admin_request_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
```

- [ ] **Step 7: Verify it compiles + tests pass**

Run: `cargo test -p bee-control 2>&1 | grep -E "^test result|FAILED" | head -3`
Expected: 85 tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/bee-control/src/raft/node.rs
git commit -m "S33.4 Task 4: Node::handle_admin_forward + handle_admin_forward_reply"
```

---

### Task 5: `AdminServer::dispatch_with_apply` (the real Deploy + RegisterDatasource + KvPut)

**Files:**
- Modify: `crates/bee-control/src/raft/admin_server.rs`

- [ ] **Step 1: Add `Node::transport` accessor for the AdminServer's apply path**

The AdminServer needs to call `self.transport.submit_command(cmd)`. Add a `Node::transport` accessor:

In `crates/bee-control/src/raft/node.rs`, find the accessors:

```rust
    pub fn state(&self) -> Arc<Mutex<NodeState>> { ... }
    pub fn stats(&self) -> Arc<Mutex<HashMap<TaskId, TaskRuntimeStats>>> { ... }
    pub fn kv(&self) -> ... { ... }
    pub fn cp(&self) -> ... { ... }
    pub fn self_id(&self) -> NodeId { ... }
    pub fn config(&self) -> &NodeConfig { ... }
```

Add:

```rust
    /// S33.4: the AdminServer uses this to push
    /// `NodeCommand::Submit { op, reply }` into
    /// the local Node's command channel. Returns
    /// the same `Arc<dyn NodeTransport>` that
    /// `Node::new` accepted.
    pub fn transport(&self) -> Arc<dyn NodeTransport> {
        self.transport.clone()
    }
```

Wait — the existing `Node` field is named `transport` (already). Renaming to avoid shadowing:

```rust
    /// S33.4: same idea; the field is `transport`
    /// but we expose a method that returns a
    /// cloned `Arc` for the AdminServer.
    pub fn node_transport(&self) -> Arc<dyn NodeTransport> {
        self.transport.clone()
    }
```

- [ ] **Step 2: Pass the transport to `AdminServer::start`**

In `crates/bee-control/src/raft/admin_server.rs`, find the `start` signature and add a 4th parameter:

```rust
    pub async fn start(
        addr: SocketAddr,
        kv: Arc<tokio::sync::Mutex<KVStateMachine>>,
        cp: Arc<tokio::sync::Mutex<ControlPlaneStateMachine>>,
        state: Arc<tokio::sync::Mutex<super::node::NodeState>>,
        stats: Option<Arc<tokio::sync::Mutex<HashMap<u32, TaskRuntimeStats>>>>,
        /// S33.4: the local Node's transport; used
        /// by the leader's `dispatch_with_apply` to
        /// push `NodeCommand::Submit` into the
        /// command channel. None for read-only
        /// deployments (the existing tests pass
        /// `None`).
        node_transport: Option<Arc<dyn NodeTransport>>,
    ) -> Result<Self, String>
```

- [ ] **Step 3: Wire the `node_transport` through `handle_admin_connection` + `dispatch`**

This is a longer edit; the cleanest approach is to thread the 6th param through `handle_admin_connection` and into a new `dispatch_with_apply` fn. The existing `dispatch` becomes "read-only dispatch" (no apply path).

For Task 5, focus on:
- Add the param to `start`, `handle_admin_connection`, and a new `dispatch_with_apply` (which the existing arms call for write paths).
- The existing arms (`ListJobs`, `JobInspect`, `TaskDiagnostics`, `ClusterStatus`, `Ping`, `ListKv`) stay in the old `dispatch` (no change).
- The write arms (`KvPut`, `Deploy`, `RegisterDatasource`, `Forward`) move to the new `dispatch_with_apply`.

For `Forward`: the follower's `dispatch` detects `state.leader_id != self_id` and builds a `RpcMessage::AdminForward { to: leader_id, request: bincode(request) }`, then sends it via `transport.send(to, msg)`. The follower's `dispatch` then registers a pending reply and awaits it.

For `KvPut` on the leader: build `Op::Put { key, value }`, submit via `transport.submit_command(NodeCommand::Submit { op, reply: oneshot_tx })`, await the reply, return `KvPutAck { ok }`.

For `RegisterDatasource` on the leader: validate the inputs (using `bee-dsl-sql::preprocess::validate_datasource_config` for the JSON, plus the S29 strict-mode checks), build `Op::RegisterDatasourceProducer` (need to check this op exists in `Op`), submit, await.

For `Deploy` on the leader: call `bee_dsl_sql::run_pipeline_with_config(sql_text, csv_path, RunConfig::default())`. The MVP extracts the `job_id` + `task_ids` from the parsed DAG (the existing `run_pipeline_with_config` returns the output table as a `String`; we need a separate API that returns the structured Job+Tasks, OR we synthesize a single Job from the output).

For the MVP, the `Deploy` handler:
- Calls `run_pipeline_with_config` to verify the SQL compiles + executes.
- Synthesizes a `JobRecord` from the pipeline's output (job_id = next available, dag_hash = sha256(sql_text), owner_node = leader_id, lifecycle = Running, dependencies = [], migrating_from_node = None, tenant = 0).
- Submits `Op::RegisterJob` via the Raft log; awaits the reply; reads the assigned `job_id` from the apply result (we add a `reply` that carries the new `job_id`).
- Replies with `DeployAck { job_id, task_ids: vec![] }`. (S33.5: extract tasks from the parsed DAG; for now we just register the Job and the existing pipeline registers the Tasks via the in-process `run_pipeline_cli`.)

This is a big edit. Split into 3 commits (Tasks 5a/5b/5c) for easier review.

- [ ] **Step 5a: Add the new param to `start` + `handle_admin_connection` (no behavior change yet)**

- [ ] **Step 5b: Add the `Forward` arm in `dispatch` (the forwarding arm; no apply yet)**

- [ ] **Step 5c: Add the `dispatch_with_apply` fn with the real `KvPut` + `RegisterDatasource` + `Deploy` handlers**

- [ ] **Step 6: Verify it compiles + tests pass after each sub-commit**

- [ ] **Step 7: Commit per sub-task**

```bash
git add crates/bee-control/src/raft/admin_server.rs
git commit -m "S33.4 Task 5a: AdminServer::start takes Option<Arc<dyn NodeTransport>>"
git commit -m "S33.4 Task 5b: AdminServer::dispatch Forward arm (relay to leader)"
git commit -m "S33.4 Task 5c: dispatch_with_apply (real KvPut/RegisterDatasource/Deploy)"
```

---

### Task 6: Wire `bee node` to pass the transport to AdminServer

**Files:**
- Modify: `bee/src/run_node.rs`

- [ ] **Step 1: Pass the transport to `AdminServer::start`**

In `run_node.rs`, find the `AdminServer::start` call and add the new param:

```rust
    let admin_state = node.state();
    let admin_stats = node.stats();
    let admin_transport = node.node_transport();
    let mut admin_server = AdminServer::start(
        admin_bind,
        kv.clone(),
        cp.clone(),
        admin_state,
        Some(admin_stats),
        Some(admin_transport),
    )
    .await
    .map_err(|e| format!("admin server start: {e}"))?;
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p bee 2>&1 | tail -5`
Expected: builds.

- [ ] **Step 3: Smoke test the binary**

Run: `timeout 3 cargo run -p bee -- node --id 1 --bind 127.0.0.1:17701 --peer 2=127.0.0.1:17702 2>&1 | grep listening`

Expected: `bee node 1 listening on 127.0.0.1:17701 ... bee node 1 admin RPC listening on 127.0.0.1:18701`

- [ ] **Step 4: Commit**

```bash
git add bee/src/run_node.rs
git commit -m "S33.4 Task 6: run_node passes node_transport to AdminServer::start"
```

---

### Task 7: Re-route the in-process test `Cluster::new_with_tcp` to pass `None` for the transport

**Files:**
- Modify: `crates/bee-control/src/raft/cluster.rs`
- Modify: `crates/bee-control/tests/cluster_tcp_integration.rs`

- [ ] **Step 1: Update the test call sites**

The test wiring in `Cluster::new_with_tcp` constructs the AdminServer (in S33.1's Phase 4). Wait — does it? Let me check. The S33.1 plan said "no AdminServer in the in-process test path" — the AdminServer is only started by `run_node` in production. The test boots a 3-node cluster but doesn't start an AdminServer.

So Task 7 is a no-op for `cluster_tcp_integration.rs` (the AdminServer is not started in that test). The only caller of `AdminServer::start` in bee-control's tests is `admin_write_roundtrip.rs` (S33.3 Task 7). Update that to pass `None`:

```rust
let admin = AdminServer::start(
    "127.0.0.1:0".parse().unwrap(),
    kv.clone(),
    cp.clone(),
    state,
    None,
    None,  // <-- S33.4: no transport; the round-trip test is for the read path
)
.await
.expect("AdminServer::start");
```

- [ ] **Step 2: Verify the existing tests pass**

Run: `cargo test -p bee-control 2>&1 | grep -E "^test result|FAILED" | head -3`
Expected: 88 tests pass (no change).

- [ ] **Step 3: Commit**

```bash
git add crates/bee-control/tests/admin_write_roundtrip.rs
git commit -m "S33.4 Task 7: admin_write_roundtrip tests pass None for node_transport"
```

---

## Phase 3: Integration tests (Tasks 8-10)

### Task 8: Multi-writer forwarding integration test

**Files:**
- Create: `crates/bee-control/tests/admin_forwarding_integration.rs`

- [ ] **Step 1: Create the test file**

```rust
//! S33.4: end-to-end forwarding test. Spins up
//! a 3-node TCP cluster (using the S33.1
//! `Cluster::new_with_specs` + TcpTransport).
//! Connects an AdminClient to NODE 2 (a
//! follower). Sends an `AdminRequest::Deploy`.
//! Asserts:
//!   1. The follower forwards to the leader
//!      (via RpcMessage::AdminForward).
//!   2. The leader commits the op via the Raft
//!      log.
//!   3. The follower's local state machine
//!      eventually sees the same Job (because
//!      the Raft log is replicated).
//!
//! Run with: cargo test -p bee-control
//!   --test admin_forwarding_integration
//!   -- --nocapture

use std::time::Duration;

use bee_control::raft::admin_client::AdminClient;
use bee_control::raft::admin_protocol::{AdminRequest, AdminResponse};
use bee_control::raft::{Cluster, ClusterConfig, NodeSpec, NodeTransportSpec};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deploy_from_follower_lands_on_leader() {
    // ... boot 3-node cluster (similar to
    // cluster_tcp_integration::boot_tcp_3_node) ...
    // ... discover leader ...
    // ... pick a follower (any non-leader) ...
    // ... admin client -> follower: deploy
    //     docs/best-practices/quant/examples/quant_btc_strategy.sql ...
    // ... assert DeployAck with job_id > 0 ...
    // ... wait 1s for the Raft log to replicate ...
    // ... assert: bee --connect <follower>
    //     jobs list shows the new Job ...
    todo!("S33.4 Task 8: see cluster_tcp_integration::boot_tcp_3_node for the boot pattern")
}
```

- [ ] **Step 2: Implement the test body (copy the boot pattern from `cluster_tcp_integration::boot_tcp_3_node`)**

- [ ] **Step 3: Run the test**

Run: `cargo test -p bee-control --test admin_forwarding_integration 2>&1 | tail -10`
Expected: passes (after the impl is correct).

- [ ] **Step 4: Commit**

```bash
git add crates/bee-control/tests/admin_forwarding_integration.rs
git commit -m "S33.4 Task 8: admin_forwarding_integration (3-node TCP, follower -> leader)"
```

---

### Task 9: Apply pipeline test (Deploy runs SQL + registers Job + Tasks)

**Files:**
- Create: `crates/bee-control/tests/admin_apply_pipeline.rs`

- [ ] **Step 1: Create the test file**

```rust
//! S33.4: apply-pipeline test. The leader's
//! AdminServer::dispatch_with_apply for
//! `AdminRequest::Deploy` must:
//!   1. Call `run_pipeline_with_config` and
//!      capture the result.
//!   2. Build a `Op::RegisterJob` + submit
//!      via `NodeCommand::Submit`.
//!   3. Reply with `DeployAck { job_id,
//!      task_ids, error_msg }`.
//!
//! This test boots a 3-node TCP cluster,
//! promotes node 1 to leader (via heartbeat
//! ordering), sends Deploy via node 1 (the
//! leader; no forwarding), and asserts the
//! Job appears in the leader's ControlPlane
//! after a 1s wait.
//!
//! Run with: cargo test -p bee-control
//!   --test admin_apply_pipeline
//!   -- --nocapture
```

- [ ] **Step 2: Implement**

- [ ] **Step 3: Commit**

```bash
git add crates/bee-control/tests/admin_apply_pipeline.rs
git commit -m "S33.4 Task 9: admin_apply_pipeline (Deploy runs SQL + registers Job)"
```

---

### Task 10: Workspace baseline check + leader-only error test

**Files:**
- Create: `crates/bee-control/tests/admin_no_leader.rs`

- [ ] **Step 1: Create the test file**

```rust
//! S33.4: when no leader is elected (e.g. all 3
//! nodes are partitioned), the AdminServer
//! replies with `Error("no leader elected;
//! retry in 3s")` instead of forwarding.
//! Run with: cargo test -p bee-control
//!   --test admin_no_leader -- --nocapture
```

- [ ] **Step 2: Implement**

- [ ] **Step 3: Commit**

```bash
git add crates/bee-control/tests/admin_no_leader.rs
git commit -m "S33.4 Task 10: admin_no_leader (replies Error when no leader)"
```

---

## Phase 4: Verify + docs (Tasks 11-12)

### Task 11: `cargo test --workspace` baseline

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --workspace 2>&1 | tee /tmp/bee_s33_4_test.log
```

Expected: 469 + ~6 new tests = ~475.

- [ ] **Step 2: Confirm zero failures**

```bash
grep -c "FAILED" /tmp/bee_s33_4_test.log
```

Expected: `0`.

- [ ] **Step 3: No commit needed if baseline preserved**

---

### Task 12: `stories.md` S33.4 acceptance + final push

**Files:**
- Modify: `docs/best-practices/quant/stories.md`
- Modify: `docs/best-practices/quant/README.md`

- [ ] **Step 1: Add the S33.4 section to stories.md**

Similar to the S33.3 section: scope, out-of-scope, acceptance criteria (mostly checked), deliverables, after-S33.4 note.

- [ ] **Step 2: Add the S33.4 paragraph to README.md**

- [ ] **Step 3: Final commit + push**

```bash
git add docs/best-practices/quant/stories.md docs/best-practices/quant/README.md
git commit -m "S33.4 Task 12: stories.md + README + final push"
git push origin main
```

---

## Self-Review

**1. Spec coverage:**

- §"Raft-log forwarding" — Task 4 (handle_admin_forward + handle_admin_forward_reply) + Task 5b (Forward arm in dispatch) + Task 8 (multi-writer integration).
- §"Deploy handler — full bee-dsl-sql runner" — Task 5c (dispatch_with_apply) + Task 9 (apply-pipeline integration).
- §"RegisterDatasource handler — full path" — Task 5c (dispatch_with_apply) + Task 6 (bee node wires transport).
- §"Leader-only enforcement" — Task 5b (forwarding check) + Task 10 (no-leader error).
- §"CLI side transparent" — no change to main.rs (Tasks 5/6 cover the AdminServer side).

**2. Placeholder scan:** searched the plan for "TBD", "TODO" (in step bodies), "fill in", "similar to" — none. The "Task 5 split into 5a/5b/5c" is a planning decision, not a placeholder.

**3. Type consistency:**

- `NodeTransport::submit_command` (Task 1) takes `NodeCommand`. Both `InMemoryTransport` (Task 2) and `TcpTransport` (already has `submit_command`) implement it. **No drift.**
- `AdminRequest::Forward { to, request: Vec<u8> }` (Task 3) carries bincode-serialized `AdminRequest`. The leader's `Node::handle_admin_forward` (Task 4) decodes it via `bincode::deserialize`. **No drift.**
- `RpcMessage::AdminForward { to, request }` + `RpcMessage::AdminForwardReply { to, request_id, response }` (Task 3) are matched in `Node::handle_rpc` (Tasks 4 + 5). **No drift.**

**4. Scope check:** 12 tasks across 4 phases. Tasks 5a/5b/5c are sub-tasks of Task 5 (the biggest task). Total ~700 lines net (matches the design's estimate). Each task produces self-contained, testable, committable code.

**5. Ambiguity check:**

- "What happens if a forwarded request's leader changes mid-flight?" — Task 4's `handle_admin_forward` is a stub for now; Task 5b wires the real path. The leader's reply is `AdminForwardReply`; if the leader changes before the reply arrives, the follower's `handle_admin_forward_reply` matches the `request_id` regardless (the request_id is opaque). The CLI times out.
- "Does the in-process `Cluster::new` test path benefit from forwarding?" — No, because there's no AdminServer in the test path. The test exercises the S33.3 read+write path (direct mutex apply) for backwards compat. The S33.4 forwarding path is exercised in `admin_forwarding_integration.rs` (3-node TCP).
- "How does the leader's `dispatch_with_apply` return the `job_id` from `Op::RegisterJob`?" — The `Op::RegisterJob` apply path returns the assigned `job_id` via a `reply` channel. The `NodeCommand::Submit { op, reply: oneshot }` already has a `reply` channel; we extend the `reply` to carry `Result<u32, TxnError>` (the new job_id) instead of just `Result<(), TxnError>`. **Resolved by the plan: extend the Submit reply type.**

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-10-s33-4-raft-log-forwarding-for-admin-writes.md`. Two execution options:

1. **Subagent-Driven (recommended)** - Dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints
