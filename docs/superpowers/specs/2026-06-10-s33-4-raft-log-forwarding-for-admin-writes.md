# S33.4 — Raft-log forwarding for admin writes (full path)

**Date:** 2026-06-10
**Type:** AFK
**Blocked by:** S33.3 (admin RPC write-path MVP)
**Status:** Approved (2026-06-10, approach B from brainstorming)

## Why this story exists

S33.3 shipped the AdminServer's write-path arms (`KvPut`, `Deploy`, `RegisterDatasource`) with a "direct local apply" simplification: the AdminServer grabs the KV / ControlPlane mutex on the receiving node and applies the op directly. This works for the 24h soak (single-writer / single-leader) but is not safe in production where:

1. A follower can receive an admin RPC and silently apply a write that the other 2 nodes don't know about (consistency violation).
2. The `Deploy` arm is a "marker only" (it doesn't actually run the SQL); the leader has no way to know which Jobs are deployed.
3. The `RegisterDatasource` arm doesn't update the per-node `DatasourceRegistry` in a way that survives a leader change.

S33.4 closes all three by:
- Funneling writes through the Raft log (every node applies the same sequence of ops, in the same order).
- Implementing the `Deploy` handler that runs `run_pipeline_with_config` and registers the resulting Job + Tasks through `Op::RegisterJob` / `Op::RegisterTask` in the Raft log.
- Implementing the `RegisterDatasource` handler that runs the existing `bee datasource create` validation + writes `Op::RegisterDatasourceProducer` to the Raft log.

## Scope

### In scope

1. **Raft-log forwarding for admin writes** (the core of S33.4)
   - The AdminServer on a non-leader node forwards `KvPut` / `Deploy` / `RegisterDatasource` to the leader via a new `AdminRequest::Forward` variant (a `RpcMessage` that carries the original `AdminRequest` payload + a `request_id`).
   - The leader handles the forwarded request through the same `dispatch` path, but instead of applying directly, it builds the appropriate `Op` and submits it to itself via `NodeCommand::Submit { op }`. The Node's run loop applies the op on the Raft log; once committed, the apply loop runs it through the KV / CP SMs.
   - The leader replies with the original `AdminResponse` shape (the client doesn't need to know forwarding happened).

2. **Deploy handler — full bee-dsl-sql runner**
   - The leader calls `run_pipeline_with_config(sql_text, csv_path, &RunConfig::default())` from `crates/bee-dsl-sql/src/physical.rs`.
   - On success, the leader constructs `Op::RegisterJob` + N × `Op::RegisterTask` from the SQL's parsed DAG (the SQL is already parsed by `run_pipeline_with_config`; the result includes the `job_id` + `task_ids`).
   - The leader submits these ops in a single `Op::Txn` so they're committed atomically.
   - On error, the leader replies with `DeployAck { job_id: 0, task_ids: [], error_msg }`.
   - The CSV path is the historical `<sql_file_basename>.csv` convention (mirrors `run_pipeline_cli`); a future S33.5 adds explicit `--csv <path>` admin support.

3. **RegisterDatasource handler — full path**
   - The leader calls the same validation path as the existing `run_datasource_cli::create` (the S29 strict-mode config check + the cdylib check + the `Datasource::new`).
   - On success, the leader constructs `Op::RegisterDatasourceProducer` (already exists in `Op`; let me verify), pushes it via `NodeCommand::Submit`.
   - On validation failure, replies with `RegisterDatasourceAck { ok: false, error_msg }`.

4. **Leader-only enforcement**
   - The AdminServer's `dispatch` reads `state.leader_id`. If `leader_id != self_id` AND the request is a write (`KvPut`, `Deploy`, `RegisterDatasource`), the AdminServer sends `AdminRequest::Forward { to: leader_id, request: <self.request> }` to the leader via the Raft channel. The leader receives and handles it.
   - If `leader_id == self_id`, the AdminServer handles the write directly (the fast path).
   - If `leader_id == None` (no leader elected yet), the AdminServer replies with `Error("no leader elected; try again in 3s")`.

5. **CLI side: the existing `bee --connect <addr> deploy / kv put / datasource create` works unchanged.** The forwarding is transparent.

### Out of scope

- File-backed KV (still 1.x).
- Cross-host clusters, TLS, mDNS / DNS-based peer discovery (1.x).
- Real P&L (the demo runs on paper / sandbox Binance keys).
- The 24h wall-clock run (HITL — human's).
- A generic `Op::Forward` variant in the Raft log itself (the forwarding is a *transport-layer* hop, not a replicated op; we use the existing `NodeCommand::Submit` to push the actual op into the Raft log).
- A request-reply correlation id for the `Forward` arm (the leader's reply goes back through the Raft channel as a new `RpcMessage::AdminForwardReply { request_id, response }`).

## Architecture

```
            ┌─────────────────────┐
            │  bee --connect CLI  │
            │  (any node)         │
            └──────────┬──────────┘
                       │ AdminRequest
                       ▼
       ┌─────────────────────────────────┐
       │  AdminServer (any node)        │
       │  ─ if read: serve locally      │
       │  ─ if write:                   │
       │     if leader_id == self:      │
       │       apply directly           │
       │     else:                      │
       │       forward to leader        │
       └──────────┬──────────────────────┘
                  │ (forwarded)
                  ▼
       ┌─────────────────────────────────┐
       │  AdminServer (leader)           │
       │  ─ submit Op::Put /            │
       │    Op::RegisterDatasource /    │
       │    Op::RegisterJob+Tasks       │
       │    via NodeCommand::Submit      │
       └──────────┬──────────────────────┘
                  │
                  ▼
       ┌─────────────────────────────────┐
       │  Raft log + apply loop          │
       │  (all 3 nodes apply the same    │
       │   sequence of ops)             │
       └─────────────────────────────────┘
```

## Wire format (new)

### `AdminRequest::Forward { to: NodeId, request: Box<AdminRequest> }`

The forwarding arm. `Box<AdminRequest>` because `AdminRequest` is non-trivial in size and we want the `Forward` arm to be a clear marker (the box enforces heap allocation; the inner request is bincode-serialized as a sub-message).

### `RpcMessage::AdminForward { to: NodeId, request: Vec<u8> }`

On the Raft channel. `Vec<u8>` is the bincode-serialized `AdminRequest`. The leader's `handle_rpc` (in `node.rs`) decodes the body and dispatches to the local `AdminServer`-equivalent path.

### `RpcMessage::AdminForwardReply { to: NodeId, request_id: u64, response: Vec<u8> }`

The leader's reply back to the follower (so the follower's `AdminServer` can send the response to the original CLI client). `request_id` correlates the reply with the original request.

## Data flow

### Write path (Deploy example; same shape for KvPut + RegisterDatasource)

1. CLI: `bee --connect <follower:8702> deploy <sql_file>` reads the .sql, sends `AdminRequest::Deploy { sql_text, owner_node: 0 }` over the admin port.
2. Follower's `AdminServer::dispatch` sees `Deploy`:
   - Reads `state.leader_id` (say = 1).
   - `to: 1`, not self → forward: `AdminRequest::Forward { to: 1, request: Box::new(self.request) }` → bincode → `RpcMessage::AdminForward { to: 1, request }` → `transport.send(1, msg)`.
3. Leader's `Node::handle_rpc` sees `AdminForward`:
   - Decodes the inner `AdminRequest`.
   - Builds a new internal `AdminServer` (or calls a free `dispatch_with_apply` fn) that handles the request, but **applies via `NodeCommand::Submit`** instead of direct mutex.
4. Leader's `dispatch_with_apply` for `Deploy`:
   - Calls `run_pipeline_with_config(sql_text, csv_path, RunConfig::default())` to compile + execute the SQL. Captures the resulting `job_id` + `task_ids`.
   - Builds `Op::Txn { ops: vec![Op::RegisterJob {...}, Op::RegisterTask {...}, ...] }`.
   - Sends `NodeCommand::Submit { op, reply: oneshot }` via `self.transport.submit_command(cmd).await`. (We add `submit_command` to `NodeTransport` trait; both `InMemoryTransport` and `TcpTransport` need an impl.)
   - Awaits the `oneshot::Receiver<()>`; on success, replies with `DeployAck { job_id, task_ids, error_msg: "" }`.
5. Leader sends `RpcMessage::AdminForwardReply { to: 2, request_id, response: bincode(DeployAck) }` back to follower.
6. Follower's `AdminServer` (which has been waiting on a `HashMap<request_id, oneshot::Sender<AdminResponse>>`) matches the reply and sends the `AdminResponse` to the original CLI client.

### Read path (unchanged from S33.3)

The read arms (`ListJobs`, `JobInspect`, `TaskDiagnostics`, `ClusterStatus`, `Ping`, `ListKv`, `KvList`) skip the forwarding check entirely and serve from the local state.

## Error handling

| Failure mode | Behavior |
|--------------|----------|
| No leader elected (`state.leader_id == None`) | Reply `Error("no leader elected; retry in 3s")` |
| Forwarded request, leader's `submit_command` returns error | Reply `Error("submit failed: <e>")` |
| `run_pipeline_with_config` returns error (SQL parse / compile / exec) | Reply `DeployAck { job_id: 0, task_ids: [], error_msg: <e> }` |
| Forwarded request, `request_id` collision (rare; 64-bit so unlikely) | Reply `Error("request_id collision; retry")` |
| Follower disconnects mid-forward | The leader's `oneshot` is dropped; the request is abandoned. The CLI's connection times out and exits non-zero. |
| `Op::Txn` rejected by `KVStateMachine::txn` (e.g. nested Txn) | Reply `Error("txn rejected: <e>")`. The Node's apply loop is robust to bad ops; it does not panic. |

## Testing strategy

- **Unit (KV)**: `KVStateMachine::txn` covers nested-Txn rejection (already exists from S07).
- **Unit (deploy)**: `run_pipeline_with_config` is well-tested in `crates/bee-dsl-sql` (S26).
- **Integration (multi-writer round-trip)**:
  - Spin up a 3-node TCP cluster.
  - From node 2 (a follower), `bee --connect <node2_admin_port> deploy <sql>`.
  - Wait for the request to round-trip to node 1 (the leader) and back.
  - Assert: `bee --connect <node1> kv list` contains the `Op::RegisterJob` entry; `bee --connect <node2>` (the follower's local state) also has the Job (because the Raft log replicated it).
  - Same for `datasource create` and `kv put`.
- **Failure (no leader)**:
  - With all 3 nodes up but the leader dead (simulate), the CLI gets `Error("no leader elected")` and exits non-zero.
- **Forwarding idempotency**:
  - Send 2 identical `kv put` requests to 2 different followers simultaneously; assert the leader's Raft log has exactly 2 `Op::Put` entries (not 4); the value is the same on all 3 nodes (the second write replaces the first, not duplicates it).

## Acceptance criteria

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` green (469 + new tests expected: ~6 forwarding + ~3 deploy + ~3 Raft-log integration)
- [ ] `bee --connect <follower:8702> deploy <sql_file>` runs the SQL on the leader and registers the Job + Tasks on all 3 nodes
- [ ] `bee --connect <follower:8702> kv put <key> <value-file>` writes to the leader's Raft log; the same key is on all 3 nodes after a 1s wait
- [ ] `bee --connect <follower:8702> datasource create <name> ...` writes a `Op::RegisterDatasourceProducer` to the Raft log; `bee --connect <any_node> jobs list` shows the datasource after a 1s wait
- [ ] When the leader is killed and a new leader is elected, all 3 nodes' KV / CP state is consistent (the Raft log is the source of truth)
- [ ] The S33.3 placeholder markers in the AdminServer are removed (the real `Deploy` + `RegisterDatasource` handlers are wired)

## Out of scope (1.x)

- File-backed KV
- Cross-host clusters, TLS, mDNS / DNS-based peer discovery
- A leader-forwarding path for the read arms (reads serve locally; a future 1.x design may forward reads too if consistency > latency)
- Real P&L
- Auto-rollback on failure
- 24h wall-clock run (HITL — human's)

## Resolutions (from brainstorming, 2026-06-10)

1. **Approach B (full path)** — Raft-log forwarding for all 3 metadata writes. The leader's `AdminServer` builds the appropriate `Op` and submits via `NodeCommand::Submit`. All 3 nodes apply the same sequence in the same order, ensuring consistency. The follower serves reads locally but forwards writes.
2. **`run_pipeline_with_config` is the Deploy runner** — the existing `crates/bee-dsl-sql` API; no new runner. CSV path is the historical `<sql_file_basename>.csv` convention.
3. **`Op::Txn` for atomic Job+Task registration** — single Raft commit covers all the `Op::RegisterJob` + `Op::RegisterTask` entries the deploy produces.
4. **Follower → leader forwarding via `RpcMessage::AdminForward`** — the Raft channel carries the forwarded request. The leader's `Node::handle_rpc` decodes + dispatches. No new RPC type at the Transport layer; we reuse the existing `send` / `recv_rpc` primitives.
5. **No new `NodeCommand` variant** — the leader uses the existing `NodeCommand::Submit { op, reply }`; the same one the in-process `bee jobs` CLI uses today.
6. **No `Box<AdminRequest>` for `Forward`** — we use a bincode-serialized `Vec<u8>` for the inner request (consistent with the rest of the wire format). The `AdminRequest::Forward { to, request: Vec<u8> }` is the actual variant; the leader's `Node::handle_rpc` deserializes it before dispatch.
7. **No-op in MVP for the deploy SQL's CSV path** — the S40 demo's 3 SQL pipelines read `<sql_file_basename>.csv` per the existing convention. The CSV file must be in the leader's working directory; the soak script's `cwd` is the bee repo root, so `examples/quant_btc_strategy.sql` looks for `examples/quant_btc_strategy.csv` (which exists in the S40 demo's tree).

## Open questions (none)

S33.4 has no remaining open questions. The 2 design clarifications from brainstorming on 2026-06-10 are recorded above.
