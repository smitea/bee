# S33.5.1 — Cross-node follower → leader forwarding (the S33.5 follow-up)

**Date:** 2026-06-10
**Type:** AFK
**Blocked by:** S33.5 (leader-side Raft-log apply)
**Status:** Approved (2026-06-10, from S33.5 spec's "Out of scope")

## Why this story exists

S33.5 shipped the leader-side `dispatch_with_apply` (writes go through the Raft log; the leader's Node run loop applies the op to its local KV / CP SM). But the follower's `dispatch(Forward)` is a placeholder: it returns "S33.5.1 wires the follower path" if the local transport is absent, and even when the local transport is present, the leader's `handle_admin_forward` hard-codes `request_id = 0` (no correlation between the follower's pending reply and the leader's reply).

S33.5.1 closes this by:
1. The follower's `dispatch(Forward)` calls `node.register_admin_reply()` to get `(request_id, oneshot::Receiver)`, embeds the `request_id` in the forwarded `RpcMessage::AdminForward`, sends via `transport.send`, and awaits the oneshot.
2. The leader's `handle_admin_forward` decodes the `request_id` from the wire, calls the admin callback, and embeds the `request_id` in the `RpcMessage::AdminForwardReply`.
3. The follower's `handle_admin_forward_reply` matches by `request_id` (already implemented in S33.4 Task 4).

The end-to-end result: a 3-node TCP cluster receives an admin write from a follower; the follower forwards to the leader; the leader commits via Raft; all 3 nodes apply consistently. The follower's CLI client gets the leader's actual response (not a placeholder).

## Scope

### In scope (3 deliverables)

1. **`AdminServer::dispatch(Forward)` — real forwarding path**
   - When the local node is **not** the leader (i.e. `state.leader_id != Some(self_id)`):
     1. Call `node.register_admin_reply()` → `(request_id, oneshot::Receiver)`.
     2. Build `RpcMessage::AdminForward { to: leader_id, request: bincode(AdminRequest::Forward { to: leader_id, request: bincode(inner) }), request_id }`.
     3. `transport.send(leader_id, msg)`.
     4. Await the `oneshot::Receiver<Vec<u8>>` (the leader's reply). The S33.5.1 timeout is 5s (configurable via `BEE_ADMIN_FORWARD_TIMEOUT_MS` env var).
     5. Build `AdminResponse::Forwarded { request_id, response: bytes }`.
   - When the local node **is** the leader: call `dispatch_with_apply` directly (S33.5's current behavior, unchanged).
   - When `state.leader_id == None` (no leader elected): reply `Error("no leader elected; retry in 3s")`.
   - When the local transport is absent (the in-process test path with `node_transport = None`): fall back to the S33.5 behavior of calling `dispatch_with_apply` directly. The test path doesn't need cross-node forwarding because there's no actual cross-node (all 3 nodes share the same in-process transport).

2. **`Node::handle_admin_forward` — extract `request_id` from the wire**
   - Add a `request_id: u64` field to `RpcMessage::AdminForward` (already exists; S33.4 added it but the leader's reply hard-codes 0).
   - The leader's `handle_admin_forward` decodes the `request_id` from the wire and uses it in the `RpcMessage::AdminForwardReply`.

3. **3-node TCP multi-writer integration test**
   - Boots 3 `bee node` processes on random ports (one per the existing `scripts/start-cluster.sh --nodes 3`).
   - Sends `AdminRequest::KvPut { key, value }` to a follower's admin RPC.
   - Asserts: the leader commits via the Raft log; all 3 nodes' `KVStateMachine` have the key after a 1s wait.
   - Same for `AdminRequest::RegisterDatasource` and `AdminRequest::Deploy`.
   - For "no leader" case: kill all 3 nodes within a 1s window; the AdminServer replies with `Error("no leader elected")`.

### Out of scope (S33.5.2, S33.5.3, 1.x)

- **S33.5.2**: full validation of `RegisterDatasource` payload (cdylib check + strict mode). The MVP uses a simple `signature = "datasource/<name>"`.
- **S33.5.3**: the full `bee-dsl-sql` runner behind `Deploy` (parses the DAG into Tasks). The MVP registers a single Job with no Tasks.
- File-backed KV (1.x).
- Cross-host clusters, TLS, mDNS / DNS peer discovery (1.x).
- Multi-leader concurrent writes (1.x; write conflicts are resolved by Raft's log ordering).
- The 24h wall-clock run (HITL — human's).

## Architecture (refined from S33.5)

```
            ┌─────────────────────┐
            │  bee --connect CLI  │
            │  (any node)         │
            └──────────┬──────────┘
                       │ AdminRequest
                       ▼
       ┌─────────────────────────────────┐
       │  AdminServer (follower)        │
       │  dispatch(Forward)              │
       │  ─ register_admin_reply()       │  ← gets (42, oneshot)
       │  ─ transport.send(leader,       │
       │    AdminForward { request_id:42,│
       │    request: bincode(Forward{...})│
       │  ─ oneshot.await ──────────┐   │
       └────────────────────────────┼──┘
                                       │
   ┌───────────────────────────────────▼──────────────────┐
   │  Node::handle_admin_forward (leader)               │
   │  ─ decode inner AdminRequest                        │
   │  ─ call admin_callback (returns AdminResponse)     │
   │  ─ send AdminForwardReply { to, request_id:42,    │
   │    response: bincode(AdminResponse) }              │
   └─────────────────────────────┬─────────────────────┘
                                  │
   ┌──────────────────────────────▼─────────────────────┐
   │  Node::handle_admin_forward_reply (follower)        │
   │  ─ matches request_id=42 in pending_admin_replies   │
   │  ─ sends response bytes to the oneshot              │
   └──────────────────────────────────────────────────────┘
```

## Wire format (refined from S33.4 / S33.5)

### `RpcMessage::AdminForward { to: u32, request: Vec<u8>, request_id: u64 }`

The `request_id` was already in the variant (S33.4 added it) but the leader's `handle_admin_forward` ignored it (hard-coded 0 in the reply). S33.5.1 threads it end-to-end.

### `RpcMessage::AdminForwardReply { to: u32, request_id: u64, response: Vec<u8> }`

Unchanged from S33.4. The `request_id` is now the *actual* id from the original `Forward` request, not a hard-coded 0.

## Data flow (refined from S33.5)

### Cross-node forward (write path)

1. CLI: `bee --connect <follower:8702> kv put <key> <file>`.
2. Follower's `AdminServer::dispatch(KvPut)`:
   - `state.leader_id` is `Some(1)`, `self_id` is `2` (not the leader).
   - `let (request_id, rx) = node.register_admin_reply().await;` → `(42, oneshot)`.
   - `let inner = bincode::serialize(&AdminRequest::KvPut {...})?;`
   - `let forward = bincode::serialize(&AdminRequest::Forward { to: 1, request: inner })?;`
   - `transport.send(1, RpcMessage::AdminForward { to: 1, request: forward, request_id: 42 }).await;`
   - `let response_bytes = rx.await?;` (the leader's reply).
   - Return `AdminResponse::Forwarded { request_id: 42, response: response_bytes }` to the CLI.
3. Leader's `Node::handle_admin_forward`:
   - Decodes the `RpcMessage::AdminForward` (gets `request_id = 42`).
   - Decodes the inner `AdminRequest::Forward { to: 1, request: bincode(KvPut) }`.
   - Decodes the inner `AdminRequest::KvPut`.
   - Calls the admin callback (which calls `dispatch_with_apply`).
   - Sends `RpcMessage::AdminForwardReply { to: 2, request_id: 42, response: bincode(KvPutAck) }` to follower.
4. Follower's `Node::handle_admin_forward_reply`:
   - Matches `request_id = 42` in the pending map.
   - Sends the response bytes to the pending `oneshot::Sender`.
5. Follower's `AdminServer::dispatch(KvPut)` resumes from `rx.await` with the response bytes.

### Local-leader path (write path)

1. CLI: `bee --connect <leader:8701> kv put <key> <file>`.
2. Leader's `AdminServer::dispatch(KvPut)`:
   - `state.leader_id` is `Some(1)`, `self_id` is `1` (the leader).
   - Calls `dispatch_with_apply` directly (S33.5's current behavior).
   - Returns `AdminResponse::KvPutAck { ok: true }`.

### Read path (no change)

`ListJobs`, `JobInspect`, etc. are served locally (no forwarding needed). Unchanged from S33.4.

## Error handling

| Failure mode | Behavior |
|--------------|----------|
| `state.leader_id == None` | `Error("no leader elected; retry in 3s")` |
| `register_admin_reply` (interior mutability lock contention) | `Error("register reply failed")` (extremely rare; only under heavy concurrent writes) |
| `transport.send` returns `TransportError` | `Error("send failed: <e>")` |
| Leader's `oneshot` reply never arrives (5s timeout) | `Error("leader reply timeout (5s)")` |
| `request_id` collision (64-bit so unlikely) | `Error("request_id collision; retry")` |
| Leader disconnected mid-flight (one of the 3 nodes is killed) | The `oneshot` is dropped when the follower's `Node::handle_admin_forward_reply` is unreachable; the `rx.await` returns `Err(_)`; the CLI's connection times out and exits non-zero |
| Forwarding loop (Leader A forwards to Leader B, etc.) | **Cannot happen** — `dispatch_with_apply` on the leader calls `dispatch_with_apply` directly when the local node is the leader (it doesn't recurse). |

## Testing strategy

- **Unit (Node)**: `Node::register_admin_reply` returns monotonically increasing ids (no collisions under 10K concurrent calls).
- **Integration (3-node TCP, multi-writer)**:
  - `cargo run -p bee -- node --id 1 --bind 127.0.0.1:7701 --peer 2=...` × 3.
  - `bee --connect <follower:8702> kv put <key> <file>`.
  - Wait 1s.
  - `bee --connect <any_node:8701> kv list <prefix>`.
  - Assert the key is on all 3 nodes.
- **Integration (no leader)**:
  - All 3 nodes running but the leader just died (mid-election).
  - The AdminServer replies `Error("no leader elected; retry")`.
- **Existing tests** (S33.1, S33.2, S33.3, S33.4, S33.5) must still pass.

## Acceptance criteria

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` green (470 + new tests expected: ~3 forwarding round-trip + ~1 no-leader + ~1 multi-writer)
- [ ] `bee --connect <follower:8702> kv put <key> <value-file>` writes to the leader's Raft log; all 3 nodes' KV have the key after a 1s wait
- [ ] `bee --connect <follower:8702> datasource create <name> ...` writes `Op::RegisterDatasourceProducer` to the Raft log; all 3 nodes' CP SMs have the Datasource
- [ ] `bee --connect <follower:8702> deploy <sql_file>` writes `Op::RegisterJob` to the Raft log; all 3 nodes' CP SMs have the Job
- [ ] The follower's CLI client receives the leader's actual `AdminResponse` (not the S33.5 placeholder "queued for leader")
- [ ] When the leader is killed, the surviving 2 nodes re-elect within 10s; subsequent writes from a follower go to the new leader

## Out of scope (1.x)

- File-backed KV
- Cross-host clusters, TLS, mDNS / DNS-based peer discovery
- Multi-leader concurrent writes (write conflicts are resolved by Raft's log ordering; for S33.5.1 we test single-writer-at-a-time)
- The full `bee-dsl-sql` runner behind `Deploy` (S33.5.3)
- The 24h wall-clock run (HITL)
- A `request_id` collision test (64-bit so unlikely)

## Resolutions (from S33.5 spec, 2026-06-10)

S33.5.1 is the natural follow-up to S33.5. The S33.5 spec's "Out of scope" section identified the cross-node forwarding + 3-node TCP test as the deferred work. S33.5.1 picks it up.

1. **`request_id` end-to-end** — the `RpcMessage::AdminForward` already carries `request_id` (S33.4 added it). The leader's `handle_admin_forward` extracts it and uses it in the reply (S33.5 hard-coded 0). The follower's `handle_admin_forward_reply` matches by `request_id` (S33.4 already implemented).
2. **The follower's `dispatch(Forward)` calls `node.register_admin_reply()`** — the API exists (S33.4 Task 4); S33.5.1 just calls it from the Forward arm.
3. **The 3-node TCP multi-writer test is the integration test** — boots a real 3-node TCP cluster; the follower → leader forwarding is end-to-end.
4. **Timeout is 5s** — `tokio::time::timeout(Duration::from_secs(5), rx.await)` wraps the oneshot await. Configurable via `BEE_ADMIN_FORWARD_TIMEOUT_MS` env var (default 5000).
5. **No forwarding loop** — the leader's `dispatch(Forward)` detects `leader_id == self_id` and calls `dispatch_with_apply` directly (no recursion). Followers' `dispatch(Forward)` only forwards to the leader, not to any peer.

## Open questions (none)

S33.5.1 has no remaining open questions. The 5 design clarifications from S33.5's "Resolutions" section are recorded above.
