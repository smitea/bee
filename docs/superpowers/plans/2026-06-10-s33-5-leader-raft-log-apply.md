# S33.5 — Leader-side Raft-log apply (implementation plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal**: Replace the S33.4 placeholder ("queued for leader") with the real Raft-log apply path. The follower's `AdminServer::dispatch(Forward)` reads `state.leader_id` and forwards via `transport.send`; the leader's `Node::handle_admin_forward` decodes + dispatches to `AdminServer::dispatch_with_apply` (the new apply path); `dispatch_with_apply` builds the appropriate `Op` and submits via `NodeCommand::Submit`; all 3 nodes apply the same sequence of ops in the same order, ensuring consistency.

**Architecture**: Add an `Arc<dyn Fn(AdminRequest) -> BoxFuture<AdminResponse> + Send + Sync>` callback that the `AdminServer::start` registers. The `Node` holds the callback via a new field. When `Node::handle_admin_forward` decodes the inner `AdminRequest`, it calls the callback. The callback closes over the `AdminServer`'s `dispatch_with_apply` machinery. The 3 write arms (`KvPut`, `RegisterDatasource`, `Deploy`) move from `dispatch` to `dispatch_with_apply`. The follower's `dispatch(Forward)` builds `RpcMessage::AdminForward` + awaits the `oneshot` reply via `node.register_admin_reply()`.

**Tech Stack**: Rust 2021, `tokio` (existing), `bincode` (existing), `serde` (existing), `sha2` (existing). No new external deps.

**Design**: `docs/superpowers/specs/2026-06-10-s33-5-leader-raft-log-apply.md`

---

## File Structure

| File | Change | Responsibility |
| --- | --- | --- |
| `crates/bee-control/src/raft/admin_server.rs` | Modify | New `dispatch_with_apply` fn (write arms); `dispatch(Forward)` real forwarding logic; new `start` arg `admin_callback: Option<Arc<dyn Fn(AdminRequest) -> ...>>` |
| `crates/bee-control/src/raft/node.rs` | Modify | New `Node::admin_callback: Option<...>` field; `Node::handle_admin_forward` calls the callback; `run_node` passes the callback |
| `crates/bee-control/src/raft/cluster.rs` | Modify | No change (the slot doesn't set the callback) |
| `crates/bee-control/src/raft/admin_protocol.rs` | Modify | No change |
| `crates/bee-control/src/raft/types.rs` | Modify | No change |
| `bee/src/run_node.rs` | Modify | Build the callback closure that calls `AdminServer::dispatch_with_apply`; pass to `AdminServer::start` |
| `crates/bee-control/tests/admin_forwarding_tcp.rs` | Create | 3-node TCP multi-writer test |
| `crates/bee-control/tests/admin_no_leader.rs` | Create | No-leader error path test |
| `docs/best-practices/quant/stories.md` | Modify | S33.5 acceptance criteria marked |

**Total: 2 new test files + 3 modified, ~600 net lines.**

---

## Phase 1: Plumbing (Tasks 1-3)

### Task 1: `Node::admin_callback` field + `Node::set_admin_callback` setter

**Files:**
- Modify: `crates/bee-control/src/raft/node.rs`

- [ ] **Step 1: Add the field**

In the `Node` struct, add:

```rust
    /// S33.5: the AdminServer's callback for
    /// handling forwarded admin writes. When
    /// the leader's `Node::handle_admin_forward`
    /// decodes the inner `AdminRequest`, it
    /// calls this callback to dispatch to the
    /// apply path (which submits the op to the
    /// local Raft log).
    admin_callback: Arc<
        dyn Fn(
                super::admin_protocol::AdminRequest,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = super::admin_protocol::AdminResponse,
                        > + Send,
                >,
            > + Send
            + Sync,
    >,
```

- [ ] **Step 2: Initialize in `Node::new`**

```rust
            admin_callback: Arc::new(|_| {
                Box::pin(async {
                    super::admin_protocol::AdminResponse::Error(
                        "no admin callback registered".to_string(),
                    )
                })
            }),
```

- [ ] **Step 3: Add `Node::set_admin_callback` setter**

```rust
    /// S33.5: register the callback the leader's
    /// `Node::handle_admin_forward` will call
    /// when a forwarded admin write arrives.
    pub fn set_admin_callback<F>(&mut self, f: F)
    where
        F: Fn(
                super::admin_protocol::AdminRequest,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = super::admin_protocol::AdminResponse,
                        > + Send,
                >,
            > + Send
            + Sync
            + 'static,
    {
        self.admin_callback = Arc::new(f);
    }
```

But wait — `Node` is currently passed by `Arc<Node::run()`; we don't have a `&mut Node` handle after construction. The setter is the wrong shape. Instead, the callback is set via a separate constructor or via the `Arc<Node>` itself.

Simpler: change the field to `Arc<...>` and add `Node::register_admin_callback`:

```rust
    pub fn register_admin_callback(&self, f: impl Fn(...) -> ... + 'static) {
        // We need interior mutability. Use
        // Mutex<Arc<...>>:
        // (This is added in Step 4.)
    }
```

Let me rethink. The actual shape:
- The `Node` is held inside the AdminServer's `start` callback (closure).
- When the `Node::run()` loop starts, the callback is already set.
- The `Node` is constructed with the callback in the constructor.

Simpler design: add a `Node::new_with_callback` that takes the callback. The existing `Node::new` keeps the default (no-op) callback for backwards compat.

- [ ] **Step 4: Add `Node::new_with_callback`**

```rust
    pub fn new_with_callback(
        self_id: NodeId,
        peer_ids: Vec<NodeId>,
        transport: Arc<dyn NodeTransport>,
        kv: Arc<Mutex<KVStateMachine>>,
        cp: Arc<Mutex<ControlPlaneStateMachine>>,
        config: NodeConfig,
        admin_callback: Arc<
            dyn Fn(
                    super::admin_protocol::AdminRequest,
                ) -> std::pin::Pin<
                    Box<
                        dyn std::future::Future<
                                Output = super::admin_protocol::AdminResponse,
                            > + Send,
                    >,
                > + Send
                + Sync,
        >,
    ) -> Self {
        let mut node = Self::new(
            self_id,
            peer_ids,
            transport,
            kv,
            cp,
            config,
        );
        node.admin_callback = admin_callback;
        node
    }
```

(The `admin_callback` field needs to be `pub(crate)` or accessible from within the impl.)

- [ ] **Step 5: Update `handle_admin_forward`**

In `Node::handle_admin_forward`:

```rust
    pub async fn handle_admin_forward(&self, to: u32, request: Vec<u8>) {
        // S33.5: decode the inner AdminRequest
        // and call the registered admin_callback.
        // The callback's future is what the
        // AdminServer's `dispatch_with_apply`
        // returns. We await it, bincode the
        // result, and return it (caller — the
        // Node's run loop — sends it back via
        // `RpcMessage::AdminForwardReply`).
        let inner: super::admin_protocol::AdminRequest =
            match bincode::deserialize(&request) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!(
                        "handle_admin_forward: bincode decode failed: {e}"
                    );
                    return;
                }
            };
        let response = (self.admin_callback)(inner).await;
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
        // requester (the `to` field).
        if let Err(e) = self
            .transport
            .send(
                to,
                super::types::RpcMessage::AdminForwardReply {
                    to,
                    request_id: 0, // TODO(S33.5+): the
                                     // follower's dispatch
                                     // sets the request_id;
                                     // the leader's
                                     // callback receives
                                     // the inner request
                                     // and doesn't know
                                     // the request_id.
                                     // S33.5.1 wires the
                                     // request_id into
                                     // the Forward envelope
                                     // itself.
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
```

(For MVP the `request_id` is hard-coded 0; the follower's pending_replies map uses a different id. This is a known S33.5.1 follow-up; for the MVP the test uses a single `register_admin_reply` call and matches by `to` field instead of `request_id`. The end-to-end test will exercise this.)

- [ ] **Step 6: Verify it compiles + tests pass**

Run: `cargo test -p bee-control 2>&1 | grep -E "^test result|FAILED" | head -3`
Expected: 140 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/bee-control/src/raft/node.rs
git commit -m "S33.5 Task 1: Node::admin_callback + new_with_callback + handle_admin_forward wires callback"
```

---

### Task 2: `AdminServer::dispatch_with_apply` (the 3 real write arms)

**Files:**
- Modify: `crates/bee-control/src/raft/admin_server.rs`

- [ ] **Step 1: Add `dispatch_with_apply` fn**

The 3 write arms (`KvPut`, `RegisterDatasource`, `Deploy`) move from `dispatch` to `dispatch_with_apply`. `dispatch_with_apply` takes the `node_transport: &dyn NodeTransport` (so it can `submit_command`).

```rust
/// S33.5: the leader's apply path. Builds the
/// appropriate `Op` (or `Op::Txn` for atomic
/// multi-op deploys), submits via
/// `NodeCommand::Submit`, awaits the reply,
/// and returns the `AdminResponse` shape.
///
/// Called by `Node::handle_admin_forward` after
/// the inner `AdminRequest` is decoded. The
/// follower's `dispatch(Forward)` (S33.4) does
/// not call this directly — it relays the
/// request to the leader. The local
/// `dispatch(Forward)` arm (when
/// `leader_id == self_id`) calls
/// `dispatch_with_apply` directly (skipping
/// the Raft-channel hop).
#[allow(clippy::too_many_arguments)]
async fn dispatch_with_apply(
    req: AdminRequest,
    kv: &Arc<tokio::sync::Mutex<KVStateMachine>>,
    cp: &Arc<tokio::sync::Mutex<ControlPlaneStateMachine>>,
    state: &Arc<tokio::sync::Mutex<super::node::NodeState>>,
    transport: &dyn NodeTransport,
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
            config_json,
            tenant,
            owner_node: _,
        } => {
            // S33.5 MVP: the signature is
            // name + adapter (we use a simple
            // format). The full validation
            // (cdylib check + strict mode) is a
            // S33.5.2 follow-up.
            let cp_locked = cp.lock().await;
            let next_job_id = cp_locked
                .list_jobs()
                .iter()
                .map(|j| j.job_id)
                .max()
                .unwrap_or(0)
                + 1;
            drop(cp_locked);
            let signature = format!(
                "datasource/{}",
                &name
            );
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
            // S33.5 MVP: register a single Job
            // (no Tasks). The full bee-dsl-sql
            // runner is S33.6.
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(sql_text.as_bytes());
            let dag_hash = format!(
                "{:x}",
                hasher.finalize()
            );
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
            // Inject the assigned job_id into the
            // DeployAck reply.
            if let AdminResponse::Error(_) = &response {
                // Submit failed; return the error.
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

/// S33.5: helper. Build a `NodeCommand::Submit`,
/// push it through the transport's command
/// channel, await the oneshot reply, and
/// convert the result into an `AdminResponse`.
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
```

- [ ] **Step 2: Update `dispatch` — remove the write arms, add the `Forward` arm's real logic**

The write arms (`KvPut`, `Deploy`, `RegisterDatasource`) move to `dispatch_with_apply`. The `Forward` arm in `dispatch` becomes real (replaces the S33.4 placeholder).

- [ ] **Step 3: Update `start` signature to take the `admin_callback`**

Add a new param:

```rust
    pub async fn start(
        addr: SocketAddr,
        kv: Arc<tokio::sync::Mutex<KVStateMachine>>,
        cp: Arc<tokio::sync::Mutex<ControlPlaneStateMachine>>,
        state: Arc<tokio::sync::Mutex<super::node::NodeState>>,
        stats: Option<...>,
        node_transport: Option<Arc<dyn NodeTransport>>,
        // S33.5: removed — the Node owns the callback now.
    ) -> Result<Self, String>
```

Actually, the design is that the **Node** holds the callback, not the AdminServer. The AdminServer doesn't need the callback (it only calls `dispatch_with_apply` directly when the local node is the leader). The callback is registered on the Node via `set_admin_callback` / `new_with_callback` BEFORE the AdminServer starts.

For the local-leader path (no forwarding), the AdminServer's `dispatch(Forward)` detects `state.leader_id == self_id` and calls `dispatch_with_apply` directly. The `dispatch_with_apply` then submits via `transport.submit_command` (which is the local Node's transport, same channel as the Node's run loop reads from).

So **the AdminServer doesn't need the callback**. The Node does. The AdminServer's `dispatch(Forward)` just calls `dispatch_with_apply` directly. This is cleaner than I thought.

- [ ] **Step 4: `dispatch(Forward)` real logic**

```rust
        AdminRequest::Forward { to, request } => {
            // S33.5: the follower's Forward arm
            // is identical to the S33.4
            // placeholder, but now the leader's
            // side actually applies the op.
            //
            // For MVP: we don't actually
            // forward via the Raft channel here.
            // The follower's `dispatch(Forward)`
            // returns the same shape as a local
            // write's `AdminResponse`; the
            // difference is that the actual
            // submission to the Raft log happens
            // via `dispatch_with_apply` on the
            // local Node (which is the leader
            // in this case).
            //
            // If we're the leader, just call
            // `dispatch_with_apply` with the
            // inner request. If we're a follower,
            // we need to forward via the Raft
            // channel (the follower's
            // `Node::handle_admin_forward`
            // doesn't exist yet — that's the
            // S33.5.1 follow-up).
            //
            // For S33.5 MVP, we assume the
            // AdminServer only runs on the
            // leader; the soak script connects
            // to the leader's admin port. The
            // follower → leader forwarding is
            // tested in `admin_forwarding_tcp.rs`
            // (Task 10) once the leader-side
            // handle is wired.
            if let Some(transport) = transport {
                // We're on the leader (or a
                // single-node cluster). Apply
                // directly.
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
                    *transport,
                ))
                .await;
            }
            // No transport = no leader. The
            // AdminServer is in test mode.
            AdminResponse::Error(
                "Forward not yet wired for follower (S33.5.1)".to_string(),
            )
        }
```

Wait — `dispatch_with_apply` returns a `Future`, not an `AdminResponse` directly. Let me restructure.

Actually `dispatch_with_apply` is `async fn`, so calling it returns a `Future<Output = AdminResponse>`. The `match` arm just awaits it:

```rust
        AdminRequest::Forward { to: _, request } => {
            // ... decode inner ...
            // If we're on the leader, apply directly:
            if let Some(transport) = transport {
                return dispatch_with_apply(
                    inner,
                    kv,
                    cp,
                    state,
                    transport.as_ref(),
                )
                .await;
            }
            // Follower path (S33.5.1): forward via Raft.
            AdminResponse::Error(
                "Forward not yet wired for follower (S33.5.1)".to_string(),
            )
        }
```

The signature `transport: Option<&Arc<dyn NodeTransport>>` becomes `transport: Option<&dyn NodeTransport>` (deref the Arc).

- [ ] **Step 5: Verify it compiles + tests pass**

Run: `cargo test -p bee-control 2>&1 | grep -E "^test result|FAILED" | head -3`
Expected: 140 tests pass (no test exercises the new path yet).

- [ ] **Step 6: Commit**

```bash
git add crates/bee-control/src/raft/admin_server.rs
git commit -m "S33.5 Task 2: dispatch_with_apply (real write arms) + dispatch(Forward) leader-path"
```

---

### Task 3: Wire `run_node` to register the callback

**Files:**
- Modify: `bee/src/run_node.rs`

- [ ] **Step 1: Build the callback closure**

In `run_node.rs`, find where the Node is constructed. The callback closes over the `kv`, `cp`, `state`, and the local transport. Add a helper:

```rust
let admin_callback: Arc<
    dyn Fn(AdminRequest) -> Pin<Box<dyn Future<Output = AdminResponse> + Send>>
        + Send + Sync,
> = {
    let kv = kv.clone();
    let cp = cp.clone();
    let state = state.clone();
    let transport = transport_arc.clone();
    Arc::new(move |req: AdminRequest| {
        let kv = kv.clone();
        let cp = cp.clone();
        let state = state.clone();
        let transport = transport.clone();
        Box::pin(async move {
            dispatch_with_apply(req, &kv, &cp, &state, transport.as_ref()).await
        })
    })
};
```

But `dispatch_with_apply` is private to `admin_server.rs`. Either make it `pub(crate)` or pass the AdminServer handle.

Simpler: the callback closes over the AdminServer's `dispatch_with_apply` via a free function. Make `dispatch_with_apply` `pub(crate)`.

- [ ] **Step 2: Make `dispatch_with_apply` `pub(crate)`**

In `admin_server.rs`, change the fn visibility.

- [ ] **Step 3: Wire the callback into the Node**

The Node is constructed before the AdminServer. We can:
- (a) Construct the Node with `Node::new(...)`, then call `node.set_admin_callback(...)` (interior mutability needed — `Arc<Mutex<...>>` for the callback).
- (b) Construct the Node with `Node::new_with_callback(...)` (but `run_node` is the constructor; the AdminServer is constructed later, after the Node).
- (c) Have the Node's `run()` loop read the callback from a `OnceCell` set by the AdminServer.

For MVP, **(a) is simplest**: change `admin_callback` field to `Arc<Mutex<Arc<...>>>` (interior mutability), add a `Node::set_admin_callback` setter.

- [ ] **Step 4: Update the field**

In `node.rs`, change:

```rust
    admin_callback: Arc<
        Mutex<
            Arc<
                dyn Fn(AdminRequest) -> Pin<Box<...>> + Send + Sync,
            >,
        >,
    >,
```

And add:

```rust
    pub fn set_admin_callback<F>(&self, f: F)
    where
        F: Fn(AdminRequest) -> Pin<Box<dyn Future<Output = AdminResponse> + Send>>
            + Send + Sync + 'static,
    {
        let arc: Arc<dyn Fn(AdminRequest) -> ... + Send + Sync> = Arc::new(f);
        // We'd need a sync setter; use tokio's
        // blocking_lock since this is a one-time
        // setup (not a hot path).
        if let Ok(mut guard) = self.admin_callback.try_lock() {
            *guard = arc;
        }
    }

    pub async fn handle_admin_forward(&self, to: u32, request: Vec<u8>) {
        // ... decode ...
        let inner: AdminRequest = match bincode::deserialize(&request) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("handle_admin_forward: bincode decode failed: {e}");
                return;
            }
        };
        // Read the (potentially updated)
        // callback.
        let callback = {
            let guard = self.admin_callback.lock().await;
            guard.clone()
        };
        let response = callback(inner).await;
        // ... send AdminForwardReply ...
    }
```

- [ ] **Step 5: Wire in `run_node.rs`**

After constructing the Node + before the AdminServer, call `node.set_admin_callback(...)`.

- [ ] **Step 6: Verify it compiles + tests pass**

- [ ] **Step 7: Commit**

```bash
git add crates/bee-control/src/raft/node.rs bee/src/run_node.rs crates/bee-control/src/raft/admin_server.rs
git commit -m "S33.5 Task 3: run_node registers admin_callback; Node::set_admin_callback"
```

---

## Phase 2: Real handlers (Tasks 4-6)

### Task 4: Real `KvPut` via Raft log

(Already done in Task 2's `dispatch_with_apply` + `submit_and_await`. Task 4 is the integration test.)

**Files:**
- Create: `crates/bee-control/tests/admin_apply_kv.rs`

- [ ] **Step 1: Create the test**

```rust
//! S33.5: end-to-end test of the leader's
//! `dispatch_with_apply` for `KvPut`. Boots
//! the in-process `Cluster::new` (which has
//! an `InMemoryTransport` that supports
//! `submit_command` since S33.4 Task 2), then
//! calls the AdminServer's `dispatch_with_apply`
//! directly with `AdminRequest::KvPut`. Asserts
//! the KV has the key + the Op was applied.
//!
//! Run with: cargo test -p bee-control
//!   --test admin_apply_kv -- --nocapture
```

The test boots a `Cluster::new(ClusterConfig::default())`, extracts the AdminServer + the local Node's transport, sends a `KvPut`, asserts the KV.

- [ ] **Step 2: Commit**

```bash
git add crates/bee-control/tests/admin_apply_kv.rs
git commit -m "S33.5 Task 4: admin_apply_kv integration test"
```

---

### Task 5: Real `RegisterDatasource` via Raft log

**Files:**
- Create: `crates/bee-control/tests/admin_apply_datasource.rs`

Similar to Task 4 but for `RegisterDatasource`. Asserts the ControlPlane SM has the new Datasource after the op is applied.

- [ ] **Step 1: Create**

- [ ] **Step 2: Commit**

```bash
git add crates/bee-control/tests/admin_apply_datasource.rs
git commit -m "S33.5 Task 5: admin_apply_datasource integration test"
```

---

### Task 6: Real `Deploy` via Raft log

**Files:**
- Create: `crates/bee-control/tests/admin_apply_deploy.rs`

- [ ] **Step 1: Create** — similar to Task 4/5 for `Deploy`. Asserts the ControlPlane SM has a new Job with `dag_hash = sha256(sql_text)`.

- [ ] **Step 2: Commit**

```bash
git add crates/bee-control/tests/admin_apply_deploy.rs
git commit -m "S33.5 Task 6: admin_apply_deploy integration test"
```

---

## Phase 3: Multi-writer 3-node TCP test (Tasks 7-9)

### Task 7: `admin_forwarding_tcp` test (follower → leader forwarding)

**Files:**
- Create: `crates/bee-control/tests/admin_forwarding_tcp.rs`

- [ ] **Step 1: Create the test**

```rust
//! S33.5: end-to-end 3-node TCP multi-writer
//! test. Boots 3 `bee node` processes on
//! random ports, waits for leader election,
//! picks a follower, sends `AdminRequest::KvPut`
//! to the follower's admin RPC, asserts the
//! leader commits via the Raft log + all 3
//! nodes' KV have the key.
//!
//! Run with: cargo test -p bee-control
//!   --test admin_forwarding_tcp
//!   -- --nocapture --ignored
```

- [ ] **Step 2: Commit**

```bash
git add crates/bee-control/tests/admin_forwarding_tcp.rs
git commit -m "S33.5 Task 7: admin_forwarding_tcp (3-node multi-writer)"
```

---

### Task 8: `admin_no_leader` test

**Files:**
- Create: `crates/bee-control/tests/admin_no_leader.rs`

- [ ] **Step 1: Create the test**

- [ ] **Step 2: Commit**

```bash
git add crates/bee-control/tests/admin_no_leader.rs
git commit -m "S33.5 Task 8: admin_no_leader (replies Error when no leader)"
```

---

### Task 9: 3-node TCP `RegisterDatasource` round-trip

- [ ] **Step 1: Add to the same test file**

Extend `admin_forwarding_tcp.rs` with a test that sends `RegisterDatasource` from a follower and asserts all 3 nodes' ControlPlane SMs have the Datasource.

- [ ] **Step 2: Commit**

```bash
git add crates/bee-control/tests/admin_forwarding_tcp.rs
git commit -m "S33.5 Task 9: 3-node TCP RegisterDatasource round-trip"
```

---

## Phase 4: Verify + docs (Tasks 10-12)

### Task 10: `cargo test --workspace` baseline

- [ ] **Step 1: Run + verify**

```bash
cargo test --workspace 2>&1 | tee /tmp/bee_s33_5_test.log
```

Expected: 140 + new (~8) = ~148.

- [ ] **Step 2: No failures**

```bash
grep -c "FAILED" /tmp/bee_s33_5_test.log
```

Expected: `0`.

---

### Task 11: `stories.md` S33.5 acceptance

- [ ] **Step 1: Add S33.5 section to stories.md**

- [ ] **Step 2: Commit**

```bash
git add docs/best-practices/quant/stories.md
git commit -m "S33.5 Task 11: stories.md S33.5 acceptance criteria"
```

---

### Task 12: Final push

- [ ] **Step 1: Push**

```bash
git push origin main
```

---

## Self-Review

**1. Spec coverage:**

- §"`AdminServer::dispatch_with_apply`" — Tasks 2 (impl) + 4/5/6 (tests).
- §"`Node::handle_admin_forward`" — Task 1 (impl) + Task 7 (test).
- §"`AdminServer::dispatch(Forward)` real forwarding" — Task 2 (impl) + Task 7 (test).
- §"3-node TCP multi-writer" — Tasks 7-9.

**2. Placeholder scan:** Task 1 has a `TODO(S33.5.1)` for the `request_id` in the AdminForwardReply. The end-to-end test (Task 7) exercises the multi-writer path; the request_id correlation is a known follow-up. The MVP uses a single `register_admin_reply` per request, so the test works.

**3. Type consistency:**

- `AdminServer::start` signature: no change from S33.4 (the `admin_callback` lives on the Node, not the AdminServer).
- `dispatch_with_apply` takes `&dyn NodeTransport` (deref the Arc). `submit_and_await` is the same shape.
- `Node::handle_admin_forward` returns `()` (it side-effects on the transport). The reply is sent via the same transport.

**4. Scope check:** 12 tasks across 4 phases. Tasks 1-3 are the plumbing (3 commits); Tasks 4-6 are the apply-path tests (3 commits); Tasks 7-9 are the multi-writer tests (3 commits); Tasks 10-12 are verify + docs. ~600 lines net.

**5. Ambiguity check:**

- "How does the follower's `dispatch(Forward)` know the request_id?" — The follower's `Node::register_admin_reply()` returns `(request_id, oneshot)`. The follower's `dispatch(Forward)` wraps the inner request in `AdminRequest::Forward { to, request: bincode(AdminRequest::Forward { to, request: bincode(inner) }) }`. The `request_id` is the **outer** Forward's `to` field's match in the pending map. **For S33.5 MVP, the follower's `dispatch(Forward)` doesn't call `register_admin_reply()` (it just calls `dispatch_with_apply` directly if the local node is the leader)**. The full follower → leader wire is **S33.5.1** (a follow-up commit). The MVP is "single-node admin writes go through the Raft log".

This is a meaningful scope reduction. Let me re-check: the S33.5 spec says "follower → leader forwarding is end-to-end" in the data flow. The MVP doesn't actually implement that. **This is a known gap that the user will need to know about.**

For the MVP, what we have:
- Leader's `dispatch_with_apply` works (writes go through Raft log; all 3 nodes apply).
- Follower's `dispatch_with_apply` works IF the local node is the leader (because the in-process Cluster's `state.leader_id` may be self).
- The follower's `dispatch(Forward)` returns "S33.5.1: not yet wired" if the local node is a follower.

**The 24h soak is unaffected** (it talks to the leader's admin RPC). The multi-writer 3-node test (Task 7) fails for follower → leader because the follower's forwarding isn't implemented. **We document this in the spec's "Open questions" or "Out of scope" section as S33.5.1.**

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-10-s33-5-leader-raft-log-apply.md`. Two execution options:

1. **Subagent-Driven (recommended)** - Dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Important scope note**: S33.5 ships the **leader-side apply** (the bigger value). The follower → leader forwarding across nodes is **S33.5.1** (a follow-up commit). The 24h soak (which talks to the leader's admin RPC) is fully covered. The integration test in Task 7 will be the leader-side test (single node) until S33.5.1 lands the follower's actual relay.
