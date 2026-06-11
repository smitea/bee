# S33.5.1 — Cross-node forwarding (implementation plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal**: Wire the follower's `dispatch(Forward)` to actually call `node.register_admin_reply()`, embed the `request_id` in the `RpcMessage::AdminForward`, and await the leader's reply. The leader's `handle_admin_forward` uses the embedded `request_id` in the reply. The end-to-end result: a 3-node TCP cluster receives an admin write from a follower; the follower forwards to the leader; the leader commits via Raft; all 3 nodes apply consistently. The follower's CLI client gets the leader's actual response (not the S33.5 placeholder).

**Architecture**: 
- `AdminServer::dispatch(Forward)` detects `state.leader_id`. If `leader_id == self_id` → call `dispatch_with_apply` directly (S33.5's behavior). If `leader_id != self_id` → call `node.register_admin_reply()` to get `(request_id, oneshot)`, build the `RpcMessage::AdminForward` with the `request_id`, send via `transport.send`, await the oneshot, return `AdminResponse::Forwarded { request_id, response }`.
- `Node::handle_admin_forward` uses the `request_id` from the wire in the `RpcMessage::AdminForwardReply`.
- 3-node TCP multi-writer integration test (boots 3 `bee node` processes, sends writes from a follower, asserts all 3 nodes have the entry).

**Tech Stack**: Rust 2021, `tokio` (existing), `bincode` (existing), `serde` (existing). No new external deps.

**Design**: `docs/superpowers/specs/2026-06-10-s33-5-1-cross-node-forwarding.md`

---

## File Structure

| File | Change | Responsibility |
| --- | --- | --- |
| `crates/bee-control/src/raft/admin_server.rs` | Modify | `dispatch(Forward)` real forwarding path (transport.send + register_admin_reply + await) |
| `crates/bee-control/src/raft/node.rs` | Modify | `handle_admin_forward` uses `request_id` from the wire |
| `crates/bee-control/tests/admin_forwarding_tcp.rs` | Create | 3-node TCP multi-writer integration test |
| `crates/bee-control/tests/admin_no_leader.rs` | Create | No-leader error path test |
| `docs/best-practices/quant/stories.md` | Modify | S33.5.1 acceptance criteria marked |

**Total: 2 new test files + 3 modified, ~400 net lines.**

---

## Phase 1: Plumb the follower's `dispatch(Forward)` (Tasks 1-2)

### Task 1: `AdminServer` constructor takes a `register_admin_reply` closure

**Files:**
- Modify: `crates/bee-control/src/raft/admin_server.rs`

- [ ] **Step 1: Add a new param to `start`**

The current `start` signature takes `node_transport: Option<Arc<dyn NodeTransport>>`. Add a second param: `register_reply: Option<Arc<dyn Fn() -> BoxFuture<'static, (u64, oneshot::Receiver<Vec<u8>>)> + Send + Sync>>`. The closure is `node.register_admin_reply` (which itself is `async`, so we wrap it in a closure that returns a `BoxFuture`).

Actually, simpler: since `node.register_admin_reply` is `async fn`, we need a closure that takes no args and returns a `BoxFuture`. The closure will be:
```rust
let register_reply: Arc<dyn Fn() -> BoxFuture<'static, (u64, oneshot::Receiver<Vec<u8>>)> + Send + Sync> = {
    let node = node_for_cb.clone();
    Arc::new(move || {
        let node = node.clone();
        Box::pin(async move { node.register_admin_reply().await })
    })
};
```

But that's hard to express without the `node` being in scope. Cleanest: **the closure is `async fn() -> (u64, oneshot::Receiver<Vec<u8>>)` stored as `Arc<dyn Fn() -> BoxFuture + Send + Sync>`**.

- [ ] **Step 2: Update the type alias**

```rust
/// S33.5.1: the closure that wraps
/// `Node::register_admin_reply` so the
/// AdminServer (which doesn't have a direct
/// `Node` handle) can request a fresh
/// `(request_id, oneshot)` pair per
/// forwarded write.
pub type AdminReplyRegistrar = Arc<
    dyn Fn() -> futures::future::BoxFuture<
            'static,
            (u64, tokio::sync::oneshot::Receiver<Vec<u8>>),
        > + Send
        + Sync,
>;
```

- [ ] **Step 3: Add the param to `start`**

```rust
pub async fn start(
    addr: SocketAddr,
    kv: ...,
    cp: ...,
    state: ...,
    stats: Option<...>,
    node_transport: Option<Arc<dyn NodeTransport>>,
    /// S33.5.1: closure that produces
    /// (request_id, oneshot::Receiver) pairs
    /// for forwarded admin writes. `None` for
    /// tests that don't exercise forwarding.
    register_reply: Option<AdminReplyRegistrar>,
) -> Result<Self, String> {
```

- [ ] **Step 4: Update the listener loop + `handle_admin_connection` to thread the new arg**

```rust
let register_reply_for_accept = register_reply.clone();
// ... in the accept loop:
tokio::spawn(async move {
    handle_admin_connection(
        conn,
        kv,
        cp,
        state,
        stats,
        transport,
        register_reply_for_accept,
    )
    .await;
});
```

And in `handle_admin_connection`:

```rust
async fn handle_admin_connection(
    mut conn: Connection,
    kv: ...,
    cp: ...,
    state: ...,
    stats: Option<...>,
    transport: Option<Arc<dyn NodeTransport>>,
    register_reply: Option<AdminReplyRegistrar>,
) { /* unchanged otherwise */ }
```

And the `dispatch` call site:

```rust
let response = dispatch(
    request,
    &cp,
    &kv,
    &state,
    stats.as_deref(),
    transport.as_deref(),
    register_reply.as_deref(),
)
.await;
```

- [ ] **Step 5: Update `dispatch`'s signature**

```rust
#[allow(clippy::too_many_arguments)]
async fn dispatch(
    req: AdminRequest,
    cp: &Arc<tokio::sync::Mutex<ControlPlaneStateMachine>>,
    kv: &Arc<tokio::sync::Mutex<KVStateMachine>>,
    state: &Arc<tokio::sync::Mutex<super::node::NodeState>>,
    stats: Option<&tokio::sync::Mutex<HashMap<u32, TaskRuntimeStats>>>,
    transport: Option<&dyn NodeTransport>,
    register_reply: Option<&AdminReplyRegistrar>,
) -> AdminResponse { /* ... */ }
```

- [ ] **Step 6: Verify it compiles + tests pass**

Run: `cargo test -p bee-control 2>&1 | grep -E "^test result|FAILED" | head -3`
Expected: builds with warnings (callers need updates).

- [ ] **Step 7: Update the test call sites in `admin_write_roundtrip.rs` and `admin_forward_smoke.rs`**

Pass `None` for `register_reply` in both test files.

- [ ] **Step 8: Commit**

```bash
git add crates/bee-control/src/raft/admin_server.rs crates/bee-control/tests/admin_write_roundtrip.rs crates/bee-control/tests/admin_forward_smoke.rs
git commit -m "S33.5.1 Task 1: AdminServer signature takes register_reply closure"
```

---

### Task 2: `dispatch(Forward)` real forwarding logic

**Files:**
- Modify: `crates/bee-control/src/raft/admin_server.rs`

- [ ] **Step 1: Replace the placeholder `Forward` arm with the real path**

```rust
        AdminRequest::Forward { to, request } => {
            // S33.5.1: the follower's real
            // forwarding path. Detect
            // leader-vs-self; either call
            // dispatch_with_apply directly (if
            // local leader) or forward to the
            // leader via the Raft channel.
            let state_locked = state.lock().await;
            let leader_id = state_locked.leader_id;
            drop(state_locked);
            let self_id = state_locked.role; // doesn't have self_id; use transport
            // Hmm, the AdminServer doesn't know
            // its own NodeId. We can read it
            // from the transport: every
            // NodeTransport impl has self_id().
            let self_id = match transport {
                Some(t) => t.self_id(),
                None => {
                    return AdminResponse::Error(
                        "Forward without transport (test mode)"
                            .to_string(),
                    );
                }
            };
            match leader_id {
                Some(leader) if leader == self_id => {
                    // Local leader. Apply directly.
                    let inner: AdminRequest =
                        match bincode::deserialize(&request) {
                            Ok(r) => r,
                            Err(e) => {
                                return AdminResponse::Error(format!(
                                    "Forward: decode failed: {e}"
                                ));
                            }
                        };
                    return Box::pin(dispatch_with_apply(
                        inner, kv, cp, state, transport.unwrap(),
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
                    // Build the wire: bincode the
                    // inner Forward with the
                    // request_id we just got.
                    let forward_envelope =
                        bincode::serialize(&AdminRequest::Forward {
                            to: leader,
                            request: request.clone(),
                        })
                        .map_err(|e| {
                            AdminResponse::Error(format!(
                                "Forward: bincode inner: {e}"
                            ))
                        })?;
                    // Send via transport.
                    if let Err(e) = transport
                        .unwrap()
                        .send(
                            leader,
                            RpcMessage::AdminForward {
                                to: leader,
                                request: forward_envelope,
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
                    return AdminResponse::Forwarded {
                        request_id,
                        response: response_bytes,
                    };
                }
                None => AdminResponse::Error(
                    "no leader elected; retry in 3s".to_string(),
                ),
            }
        }
```

Wait — `self_id` is not on `state_locked`. Let me re-look:

The Node's `state.lock().await.role` is `Role`, not `NodeId`. The Node's `self_id` is a separate field on `Node` (not on `NodeState`). The AdminServer can read it via the transport's `self_id()` method. That's what I wrote.

But there's a borrow issue: I do `let self_id = match transport { ... }` then later `transport.unwrap()` — the match consumed transport. Let me fix:

```rust
let self_id = transport.map(|t| t.self_id());
// ... later:
let transport = transport.expect("checked above");
```

Or use a different approach. Simplest: pass `self_id` as a new param to `dispatch`. But that's another signature change.

Cleanest: read it once and store:

```rust
let self_id = match transport {
    Some(t) => t.self_id(),
    None => {
        return AdminResponse::Error(
            "Forward without transport (test mode)".to_string(),
        );
    }
};
```

Then later: `let transport = transport.expect("checked above");`.

- [ ] **Step 2: Update `node.rs::handle_admin_forward` to use the wire `request_id`**

```rust
    pub async fn handle_admin_forward(
        &self,
        to: u32,
        request: Vec<u8>,
        request_id: u64,  // S33.5.1: was hard-coded 0
    ) {
        // ... decode inner ...
        // ... call callback ...
        // ... bincode response ...
        // ... send AdminForwardReply with the actual request_id ...
        if let Err(e) = self.transport.send(
            to,
            RpcMessage::AdminForwardReply {
                to,
                request_id,  // was 0
                response: response_bytes,
            },
        ).await {
            eprintln!(...);
        }
    }
```

The handle_rpc match arm that calls `handle_admin_forward` extracts the `request_id` from the RpcMessage:

```rust
            RpcMessage::AdminForward { to, request, request_id } => {
                self.handle_admin_forward(to, request, request_id).await;
            }
```

- [ ] **Step 3: Verify it compiles + tests pass**

- [ ] **Step 4: Commit**

```bash
git add crates/bee-control/src/raft/admin_server.rs crates/bee-control/src/raft/node.rs
git commit -m "S33.5.1 Task 2: dispatch(Forward) real path + handle_admin_forward threads request_id"
```

---

## Phase 2: 3-node TCP multi-writer test (Tasks 3-4)

### Task 3: `admin_forwarding_tcp` — 3-node multi-writer test

**Files:**
- Create: `crates/bee-control/tests/admin_forwarding_tcp.rs`

- [ ] **Step 1: Create the test file**

The test boots 3 `bee node` processes (similar to `cluster_tcp_integration.rs::boot_tcp_3_node`). Then:
- Pick a follower (any non-leader).
- Send `AdminRequest::KvPut` to the follower's admin RPC.
- Wait 1s.
- Connect to each node, send `AdminRequest::ListKv` with `prefix = "<key>"`, assert each returns the entry.

The test uses the `AdminClient` + `Cluster::new` patterns. The 3-node TCP cluster is the S33.1 `TcpTransport` boot.

- [ ] **Step 2: Run the test**

Run: `cargo test -p bee-control --test admin_forwarding_tcp -- --nocapture --ignored`
Expected: passes.

- [ ] **Step 3: Commit**

```bash
git add crates/bee-control/tests/admin_forwarding_tcp.rs
git commit -m "S33.5.1 Task 3: admin_forwarding_tcp (3-node multi-writer KvPut)"
```

---

### Task 4: `admin_no_leader` test

**Files:**
- Create: `crates/bee-control/tests/admin_no_leader.rs`

- [ ] **Step 1: Create**

Boots 3 nodes; kills all 3 in a tight window (before a new leader is elected); the AdminServer replies `Error("no leader elected")`.

- [ ] **Step 2: Commit**

```bash
git add crates/bee-control/tests/admin_no_leader.rs
git commit -m "S33.5.1 Task 4: admin_no_leader (replies Error when no leader)"
```

---

## Phase 3: Verify + docs (Tasks 5-6)

### Task 5: `cargo test --workspace` baseline

- [ ] **Step 1: Run**

```bash
cargo test --workspace 2>&1 | tee /tmp/bee_s33_5_1_test.log
```

Expected: 470 + new (~3) = ~473.

- [ ] **Step 2: No failures**

```bash
grep -c "FAILED" /tmp/bee_s33_5_1_test.log
```

Expected: `0`.

---

### Task 6: `stories.md` S33.5.1 acceptance + final push

**Files:**
- Modify: `docs/best-practices/quant/stories.md`

- [ ] **Step 1: Add the S33.5.1 section**

- [ ] **Step 2: Commit + push**

```bash
git add docs/best-practices/quant/stories.md
git commit -m "S33.5.1 Task 6: stories.md S33.5.1 acceptance + final push"
git push origin main
```

---

## Self-Review

**1. Spec coverage:**

- §"`AdminServer::dispatch(Forward)` real forwarding" — Tasks 1-2.
- §"`Node::handle_admin_forward` extracts `request_id`" — Task 2.
- §"3-node TCP multi-writer test" — Task 3.
- §"no-leader test" — Task 4.

**2. Placeholder scan:** No `TBD` / `TODO` in step bodies.

**3. Type consistency:**

- `AdminReplyRegistrar` is a new type alias for the closure.
- `dispatch` takes `Option<&AdminReplyRegistrar>` (the 7th arg). Same signature shape as the existing 6.
- `handle_admin_forward` takes `request_id: u64` as a new arg (was hard-coded 0).

**4. Scope check:** 6 tasks across 3 phases. ~400 lines net. Each task is a focused unit.

**5. Ambiguity check:**

- "What if the `register_reply` closure is called concurrently from multiple `dispatch(Forward)` arms?" — `Node::register_admin_reply` is `async fn`; the closure is `Arc<dyn Fn() -> BoxFuture + Send + Sync>`. The Node's `next_admin_request_id: AtomicU64` is thread-safe. The `pending_admin_replies: Arc<Mutex<HashMap<...>>>` is also thread-safe. Concurrent calls are fine.
- "What if the leader's reply has a different `request_id` than the follower expected?" — The follower's `Node::handle_admin_forward_reply` matches by `request_id`. A mismatch (e.g. due to a bug) just leaves the entry in the map forever; the test's short timeout catches it.
- "What about the AdminClient's `Forwarded` response handling?" — The existing `AdminClient::call` returns `Err(AdminError::ServerError(_))` if the reply is `AdminResponse::Error(_)`. The `Forwarded` variant is **not** the wire shape the AdminClient expects — the AdminClient's `match resp` in `admin_write_roundtrip.rs` doesn't know about `Forwarded`. **The CLI's `bee --connect ... kv put` would fail with "unexpected response: Forwarded(...)"**.

This is a problem. Let me think.

Actually, looking at the AdminClient more carefully: the CLI sends `AdminRequest::KvPut` (not `AdminRequest::Forward`). The CLI's flow is: `bee --connect <leader> kv put ...` → `AdminClient::call(AdminRequest::KvPut)`. The AdminServer on the **leader** receives `AdminRequest::KvPut`, the `dispatch` arm is `KvPut` directly (not `Forward` because the leader's local path doesn't go through `Forward`). So the CLI never sends `Forward`. **The `Forward` arm is only used internally when a *follower's* AdminServer gets `Forward` from another follower** — which doesn't happen in the CLI's path.

The CLI's `--connect` always connects to the **leader's** admin RPC (the discover-leader path in `start-cluster.sh` + the soak script's Phase 2). The CLI's `bee --connect <follower>` is only for diagnostic purposes. The follower's admin RPC, when it receives `KvPut`, detects `state.leader_id != self_id` and forwards internally — the CLI never sees `Forward`.

So the `Forward` arm is **internal to the AdminServer's own dispatch** (when a follower receives a write on its admin port). The CLI's response is the leader's `AdminResponse` (e.g. `KvPutAck`), wrapped in `AdminResponse::Forwarded { request_id, response: bincode(KvPutAck) }`. The AdminClient doesn't know about `Forwarded` — it returns the inner `AdminResponse::Forwarded` as-is (since it's an enum variant), and the CLI's match fails.

**This is a real issue**. The fix: the AdminClient should unwrap `Forwarded` → recurse into the inner `response` bytes. Or the AdminServer on the leader should NOT return `Forwarded` but the actual response (the follower's dispatch unwraps before returning to the CLI).

The cleanest: the follower's `dispatch(Forward)` **unwraps the response bytes** before returning to the AdminClient. Instead of `AdminResponse::Forwarded { request_id, response: bytes }`, the follower's dispatch deserializes `bytes` and returns the inner `AdminResponse`. The CLI gets the leader's actual `AdminResponse`.

- [ ] **Step 5: Update Task 2 to unwrap `Forwarded` in the AdminServer (not the AdminClient)**

```rust
// At the end of the Forward arm's forward path:
let response_bytes = ...; // from rx.await
let inner_response: AdminResponse = match bincode::deserialize(&response_bytes) {
    Ok(r) => r,
    Err(e) => {
        return AdminResponse::Error(format!(
            "Forward: decode leader reply: {e}"
        ));
    }
};
inner_response
```

The follower's `dispatch(Forward)` returns the leader's actual `AdminResponse` (e.g. `KvPutAck`) directly. The AdminClient is unchanged.

- [ ] **Step 6: Update the Step 1 code block (and the plan) accordingly**

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-10-s33-5-1-cross-node-forwarding.md`. Two execution options:

1. **Subagent-Driven (recommended)** - Dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints
