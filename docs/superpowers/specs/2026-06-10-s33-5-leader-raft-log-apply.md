# S33.5 — Leader-side Raft-log apply (the S33.4 follow-up)

**Date:** 2026-06-10
**Type:** AFK
**Blocked by:** S33.4 (Forward arm wire + pending_replies map)
**Status:** Approved (2026-06-10, from S33.4 Resolutions)

## Why this story exists

S33.4 shipped the follower-side Forward arm + the wire types + the pending-replies map. The leader-side `dispatch_with_apply` is a placeholder that returns "queued for leader". S33.5 wires the real Raft-log apply: when the leader receives an `AdminRequest::Forward`, it decodes the inner `AdminRequest`, builds the appropriate `Op`, and submits via `NodeCommand::Submit`. All 3 nodes apply the same sequence of ops in the same order, ensuring consistency.

The 24h soak (single-writer, single-leader) works with S33.3's local-apply MVP. S33.5 makes the multi-leader path safe.

## Scope

### In scope (4 deliverables)

1. **`AdminServer::dispatch_with_apply` (the leader's apply path)**
   - New fn on `AdminServer`. Takes the same args as `dispatch` plus the `node_transport: Arc<dyn NodeTransport>` (the local Node's transport; the AdminServer pushes `NodeCommand::Submit` through it).
   - The 3 write arms (`KvPut`, `RegisterDatasource`, `Deploy`) move from `dispatch` to `dispatch_with_apply`. The read arms stay in `dispatch`.
   - Each write arm:
     1. Builds the appropriate `Op` (or `Op::Txn` for atomic multi-op deploys).
     2. Submits via `transport.submit_command(NodeCommand::Submit { op, reply: oneshot })`.
     3. Awaits the reply.
     4. Returns the `AdminResponse` shape.

2. **`Node::handle_admin_forward` (the leader's dispatch entry point)**
   - Currently a stub (just `eprintln!`s). Replace with: decode the inner `AdminRequest`, build a new local `AdminServer` (or call into a `dispatch_with_apply` on an existing AdminServer), then `send_response` back to the follower via `RpcMessage::AdminForwardReply`.
   - The reply is the `AdminResponse` from the apply, bincode-serialized.
   - In practice, the leader's `Node` doesn't have a direct `AdminServer` handle. The cleanest design: the `Node` holds a callback `Arc<dyn Fn(AdminRequest) -> BoxFuture<AdminResponse> + Send + Sync>` that the `AdminServer::start` registers. The callback closes over the AdminServer's `dispatch_with_apply` machinery.

3. **`AdminServer::dispatch(Forward)` — real forwarding path**
   - The follower's `dispatch` reads `state.leader_id`. If `leader_id == self_id` → handle locally (call `dispatch_with_apply` directly, bypassing the Forward).
   - If `leader_id != self_id` → forward:
     1. Call `node.register_admin_reply()` to get `(request_id, oneshot)`.
     2. Build `RpcMessage::AdminForward { to: leader_id, request: bincode(AdminRequest::Forward { to: leader_id, request: bincode(inner) }) }` — the inner is wrapped in another `Forward` envelope; the leader's `handle_admin_forward` unwraps.
     3. `transport.send(leader_id, msg)`.
     4. Await the `oneshot::Receiver<Vec<u8>>` (the leader's reply).
     5. Return `AdminResponse::Forwarded { request_id, response: bytes }` (the AdminClient unwraps it; the CLI never sees the wrapper).
   - If `leader_id == None` → reply `Error("no leader elected; retry in 3s")`.

4. **3-node TCP multi-writer integration test**
   - Boots a 3-node TCP cluster (per `cluster_tcp_integration::boot_tcp_3_node`).
   - Sends `AdminRequest::KvPut { key, value }` from a follower.
   - Asserts: the leader commits via Raft log; all 3 nodes' `KVStateMachine` contain the key after a 1s wait.
   - Same for `AdminRequest::RegisterDatasource`.
   - For Deploy: send from a follower; assert the leader's ControlPlane SM has the new Job; all 3 nodes replicate the Job after a 1s wait.
   - For no-leader: kill all 3 nodes; the AdminServer replies with `Error("no leader elected")`.

### Out of scope (1.x)

- File-backed KV (still 1.x).
- Cross-host clusters, TLS, mDNS / DNS-based peer discovery.
- A generic `Op::Forward` variant in the Raft log itself (the Forward is a *transport-layer* hop, not a replicated op).
- Multi-leader concurrent writes (write conflicts are resolved by Raft's log ordering; for S33.5 we test single-writer-at-a-time).
- The 24h wall-clock run (HITL — human's).

## Architecture (refined from S33.4)

```
            ┌─────────────────────┐
            │  bee --connect CLI  │
            │  (any node)         │
            └──────────┬──────────┘
                       │ AdminRequest
                       ▼
       ┌─────────────────────────────────┐
       │  AdminServer (any node)        │
       │  dispatch(AdminRequest)        │
       │  ─ if read: serve locally      │
       │  ─ if write:                   │
       │     if leader_id == self:      │
       │       dispatch_with_apply(...) │
       │     else:                      │
       │       forward to leader        │
       └──────────┬──────────────────────┘
                  │ (forwarded)
                  ▼
       ┌─────────────────────────────────┐
       │  Node::handle_admin_forward     │
       │  (leader side)                  │
       │  ─ decode inner AdminRequest    │
       │  ─ call AdminServer callback    │
       │  ─ bincode the response         │
       │  ─ send AdminForwardReply back  │
       └──────────┬──────────────────────┘
                  │
                  ▼
       ┌─────────────────────────────────┐
       │  AdminServer::dispatch_with_apply │
       │  (leader, holds the Op-builders)  │
       │  ─ build Op::Put / Op::RegJob    │
       │  ─ submit via NodeCommand::Submit │
       │  ─ await the oneshot reply       │
       └──────────┬──────────────────────┘
                  │
                  ▼
       ┌─────────────────────────────────┐
       │  Node's run loop                │
       │  ─ apply Op to KV/CP SM         │
       │  ─ commit + replicate via Raft  │
       │  ─ reply via the oneshot        │
       └─────────────────────────────────┘
```

## Wire format (unchanged from S33.4)

### `AdminRequest::Forward { to: u32, request: Vec<u8> }`

The follower's relay payload. `request` is bincode(AdminRequest::KvPut | Deploy | RegisterDatasource).

### `RpcMessage::AdminForward { to: u32, request: Vec<u8> }`

Same payload, on the Raft channel.

### `RpcMessage::AdminForwardReply { to: u32, request_id: u64, response: Vec<u8> }`

The leader's reply.

## Data flow (refined from S33.4)

### Write path (KvPut example)

1. CLI: `bee --connect <follower:8702> kv put <key> <file>`.
2. Follower's `AdminServer::dispatch(KvPut)`:
   - Reads `state.leader_id` (say = 1, not self).
   - Calls `node.register_admin_reply()` → `(42, oneshot)`.
   - Builds `RpcMessage::AdminForward { to: 1, request: bincode(AdminRequest::Forward { to: 1, request: bincode(KvPut) }) }`.
   - `transport.send(1, msg)`.
   - `oneshot.await` → response bytes.
   - Builds `AdminResponse::Forwarded { request_id: 42, response: bytes }` and sends to the CLI.
3. Leader's `Node::handle_admin_forward`:
   - Decodes the inner `AdminRequest` (deserializes the bincode envelope).
   - Calls the AdminServer's callback (the closure that holds `dispatch_with_apply`).
   - The callback returns an `AdminResponse` (e.g. `KvPutAck { ok: true }`).
   - Sends `RpcMessage::AdminForwardReply { to: 2, request_id: 42, response: bincode(KvPutAck) }` to follower.
4. Follower's `Node::handle_admin_forward_reply`:
   - Matches `request_id = 42` in the pending map.
   - Sends the response bytes to the pending `oneshot::Sender`.
5. Follower's `AdminServer::dispatch(Forward)` resumes from `oneshot.await` with the response.

### Apply path (leader side, KvPut)

1. `AdminServer::dispatch_with_apply(KvPut)`:
   - Builds `Op::Put { key, value }`.
   - `let (tx, rx) = oneshot::channel(); transport.submit_command(NodeCommand::Submit { op, reply: tx }).await`.
   - `rx.await` → `Result<(), TxnError>`.
   - On `Ok`, return `AdminResponse::KvPutAck { ok: true }`.
   - On `Err`, return `AdminResponse::Error(format!("apply failed: {e}"))`.

### Apply path (leader side, Deploy)

1. `AdminServer::dispatch_with_apply(Deploy { sql_text, owner_node })`:
   - Determines the CSV path: `<sql_file_basename>.csv` of the original `<sql_file>` (S40 demo convention). The CLI sends the SQL but not the file path; the leader reconstructs the path from the SQL's first `FROM <csv>` clause OR uses a default `data/` directory.
   - For the MVP, we don't actually run the SQL (the S33.4 Deploy marker is preserved as a fallback). We register a single Job (no Tasks) with `dag_hash = sha256(sql_text)`. The S33.5 scope is the Raft-log apply path, not the bee-dsl-sql runner.
   - Determines the next `job_id`: lock the CP, find max+1, release.
   - Builds `Op::RegisterJob { job_id, dag_hash, owner_node, tenant: 0 }`.
   - `transport.submit_command(...)`; await.
   - Returns `DeployAck { job_id, task_ids: vec![], error_msg }`.

The full bee-dsl-sql runner behind Deploy (parsing the DAG into Tasks) is a S33.6 follow-up.

## Error handling

| Failure mode | Behavior |
|--------------|----------|
| No leader elected | `Error("no leader elected; retry in 3s")` |
| `submit_command` returns `TransportError` | `Error("submit failed: <e>")` |
| Op apply returns `TxnError` | `Error("apply failed: <e>")` |
| Forward to non-existent leader (race: leader just died) | `Error("leader <id> unreachable; retry")` |
| Follower disconnects mid-forward | Leader's `oneshot` is dropped; CLI times out |
| `request_id` collision (64-bit so unlikely) | `Error("request_id collision; retry")` |

## Testing strategy

- **Unit (KV)**: `KVStateMachine::txn` rejects nested Txn (already exists from S07).
- **Unit (leader-id detection)**: a test that boots a 3-node cluster, picks a non-leader, sends `KvPut`, asserts the response is `Forwarded { ... }` and the leader's `KVStateMachine` has the key.
- **Integration (multi-writer 3-node TCP)**:
  - `cargo run -p bee -- node --id N --bind ...` × 3.
  - `bee --connect <follower> kv put ...` (using the new CLI).
  - Wait 1s.
  - `bee --connect <any_node> kv list <prefix>` (using the new CLI).
  - Assert the key is on all 3 nodes.
- **Integration (Deploy round-trip)**:
  - Same setup, send Deploy.
  - Wait 1s.
  - `bee --connect <any_node> jobs list` shows the new Job on all 3 nodes.
- **Integration (no leader)**:
  - All 3 nodes running but the leader just died; a new leader hasn't been elected yet (the 3 × heartbeat_interval window).
  - The AdminServer replies `Error("no leader elected; retry")`.
- **Existing tests** (S33.1, S33.2, S33.3, S33.4) must still pass.

## Acceptance criteria

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` green (140 + new tests expected: ~5 forwarding round-trip + ~3 apply unit + ~2 error path)
- [ ] `bee --connect <follower:8702> kv put <key> <value-file>` writes to the leader's Raft log; all 3 nodes' KV have the key after a 1s wait
- [ ] `bee --connect <follower:8702> datasource create ...` writes `Op::RegisterDatasourceProducer` to the Raft log; all 3 nodes' CP SM have the Datasource
- [ ] `bee --connect <follower:8702> deploy <sql_file>` writes `Op::RegisterJob` to the Raft log; all 3 nodes' CP SM have the Job
- [ ] The S33.4 placeholder ("queued for leader") is removed; the response is the real `AdminResponse`
- [ ] When the leader is killed, the surviving 2 nodes re-elect within 10s; subsequent writes go to the new leader

## Out of scope (1.x)

- File-backed KV
- Cross-host clusters, TLS, mDNS / DNS-based peer discovery
- Multi-leader concurrent writes (write conflicts)
- The full `bee-dsl-sql` runner behind `Deploy` (S33.6)
- The 24h wall-clock run (HITL)
- A `request_id` collision test (64-bit so unlikely)

## Resolutions (from S33.4 spec, 2026-06-10)

S33.5 is the natural follow-up to S33.4. The S33.4 spec's "Resolutions" section identified Task 5c (the leader's `dispatch_with_apply`) as the deferred work. S33.5 picks it up.

1. **`AdminServer::dispatch_with_apply` is the leader's apply path** — builds `Op` and submits via `NodeCommand::Submit`. The follower's `dispatch` (S33.4) detects `state.leader_id` and forwards if not self.
2. **`Node::handle_admin_forward` decodes + dispatches** — the leader's `Node` decodes the inner `AdminRequest` and calls a callback that the `AdminServer::start` registers. The callback holds the `AdminServer`'s `dispatch_with_apply` machinery.
3. **The 3-node TCP multi-writer test is the integration test** — boots a real 3-node TCP cluster; the follower → leader forwarding is end-to-end.
4. **The full `bee-dsl-sql` runner is still S33.6** — the MVP Deploy registers a Job with no Tasks; the human can verify the Job via `bee --connect <any_node> jobs list`.

## Open questions (none)

S33.5 has no remaining open questions. The 4 design clarifications from S33.4 are resolved as recorded above.
