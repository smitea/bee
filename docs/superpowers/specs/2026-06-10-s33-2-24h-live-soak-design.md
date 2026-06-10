# S33.2 — 24h live soak (design)

**Date:** 2026-06-10
**Type:** HITL (24h wall-clock, real credentials, real Binance / NewsAPI / InfluxDB / MongoDB)
**Blocked by:** S33.1 (multi-node + failover demo)
**Status:** Approved (2026-06-10, all 5 design sections)

## Why this story exists

The S33 sign-off form's 4 production-deployment rows are
unverifiable without a 24h wall-clock run. S33.1 closed the
"Failover verified" row by adding the 3-node multi-process
plumbing. S33.2 closes the remaining 3 rows:

1. **Real money signals observed** — Binance WS flowing for ≥ 24h
2. **InfluxDB data verified** — `klines` measurement has live data
3. **MongoDB data verified** — `trades` collection has live data

The agent's role is to ship the **reproducible 24h soak
procedure** — the script, the monitoring loop, the success
criteria, the human-fillable results template. The actual
24h wall-clock run is the human's.

## Scope

### In scope (4 + 1 deliverables)

1. **AdminServer-into-run_node** (S33.1 follow-up)
   - `bee node` starts an `AdminServer` on a separate port
     (default = raft_port + 1000; `--admin-bind <addr>`
     overrides).
   - Graceful shutdown: SIGINT/SIGTERM sends `Shutdown` to
     the AdminServer's accept loop, drops the listener.
   - The 5 `Arc<dyn Fn() -> X + Send + Sync>` closures
     (role / term / commit_index / log_length / leader_id)
     in `AdminServer::start` are replaced with a single
     `Arc<Mutex<NodeState>>` parameter — the dispatch loop
     awaits the lock instead of running a sync closure. This
     removes the S33.1 5-closure hack that the spec
     acknowledged was "for clean ownership, not necessity".

2. **TaskRuntimeStats** (new field on TaskDiagDetail)
   ```rust
   pub struct TaskRuntimeStats {
       pub messages_processed: u64,    // cumulative
       pub messages_per_sec: f64,      // 1-min rolling avg
       pub last_message_at_ms: u64,    // unix epoch ms; 0 = never
       pub error_count: u64,           // cumulative
       pub last_error_at_ms: u64,      // unix epoch ms; 0 = never
       pub last_error: Option<String>, // truncated to 1KB
   }
   ```
   - Data source: `HandlerVtable::report_stats` hook in
     `bee-runtime`. After each `invoke()` call, the handler
     reports `(messages_delta, error_delta, last_error?)`
     to the Node via the existing PluginAdapter.
   - Storage: in-memory `Arc<Mutex<HashMap<TaskId,
     TaskRuntimeStats>>>` on the Node. Restart = fresh stats
     (acceptable for the 24h soak; file-backed is 1.x).
   - Wire: `AdminRequest::TaskDiagnostics(id)` returns
     `AdminResponse::TaskDiag(Some(TaskDiagDetail {
     ..., runtime_stats: Some(TaskRuntimeStats) }))`.
   - MVP mock plugins (binance / google_news / etc.) get
     manual instrumentation in this story. Production
     plugins (S34–S39) inherit the hook in their next
     release.

3. **KVStateMachine::list + Op::List** (new)
   - `pub fn list(&self, prefix: &str) -> Vec<(String,
     Vec<u8>)>` — O(n) scan of the in-memory HashMap,
     filter by key prefix. For 24h soak = 288 entries max,
     this is fine.
   - `Op::List { prefix: String }` variant. `apply_op` calls
     `kv.list(&prefix)` and returns the result via a new
     `OpResult::List(Vec<(String, Vec<u8>)>)` enum (or
     pushes the result to a per-Op reply channel — TBD in
     the plan).
   - **Wire**: `AdminRequest::ListKv { prefix: String }` →
     `AdminResponse::KvList(Vec<(String, Vec<u8>)>)`. This
     is a *read* so it bypasses the Raft log (each Node
     serves from its local KV; the result is consistent
     within a single read but the prefix scan over a
     288-entry key space is small enough that stale reads
     across the 24h loop are not a concern).
   - CLI: `bee --connect <addr> kv list <prefix>` prints
     `(key, value_len) per line; --raw prints the full
     bincode body per line.

4. **`scripts/soak-quant-24h.sh`** — the 24h monitoring loop
   - **Phase 0 — setup**: source `scripts/.env`; build 6
     production plugins (`cargo build --release -p
     bee-plugin-*`); stage cdylibs to
     `/tmp/bee_prod_plugins/`.
   - **Phase 1 — start cluster**: `scripts/start-cluster.sh
     --nodes 3` (3 `bee node` workers on 127.0.0.1:7701-3,
     admin 127.0.0.1:8701-3).
   - **Phase 2 — discover leader**: poll each admin port
     until `cluster status` returns a valid `leader_id`.
     Record `LEADER_ADDR` (e.g. `127.0.0.1:8701`) for the
     rest of the script.
   - **Phase 3 — register datasources**: 4× `bee --connect
     $LEADER datasource create <name> --adapter <a>
     --plugin-version <v> --config <json>`.
   - **Phase 4 — deploy pipelines**: 3× `bee --connect
     $LEADER deploy <sql_file>`.
   - **Phase 5 — bootstrap check (5 min)**: every 30s, query
     InfluxDB `klines` count + MongoDB `trades` count via
     curl / mongosh. At T+5min, assert both are > 0. If
     not, exit 1 with a clear "deploy did not produce
     data" error.
   - **Phase 6 — human gate**: print "Bootstrap OK. Hit
     Enter to start 24h loop, Ctrl-C to abort." Human
     decides.
   - **Phase 7 — 24h monitoring loop** (or `--smoke`: 5 min
     + 5s interval):
     ```
     while elapsed < 24h:
         # Per-tick snapshot
         metrics = {}
         metrics['ts'] = now_iso()
         metrics['elapsed_sec'] = elapsed
         metrics['cluster'] = bee --connect $LEADER cluster status
         metrics['jobs'] = bee --connect $LEADER jobs
         metrics['tasks'] = []
         for task_id in all task_ids:
             metrics['tasks'].append(
                 bee --connect $LEADER diagnostics $task_id
             )
         metrics['influx_klines_per_min'] =
             curl InfluxDB query (last 5m, count)
         metrics['mongo_trades_per_min'] =
             mongosh aggregation (last 5m, count)

         # Threshold checks
         if any node log_lag > 1000: exit 1
         if any task Orphaned for > 60s: exit 1
         if influx_klines_per_min == 0 for last 10 min:
             exit 1
         if mongo_trades_per_min == 0 for last 10 min:
             exit 1

         # Persist tick to leader KV
         bee --connect $LEADER kv put
             soak/run_$RUN_ID/tick_$TS
             <bincode(metrics)>

         # Sleep 5 min (or 5s for --smoke)
     ```
   - **Phase 8 — `--failover-midway` injection** (only if
     flag set): at T+12h, `scripts/kill-node.sh --node 2`
     (already in tree). Record `failover_at_ms` and
     `recovered_at_ms` (the first tick where cluster status
     reports a leader ≠ 2).
   - **Phase 9 — summary**: print per-hour table with
     rows (hour | influx_klines_total | mongo_trades_total
     | per-node role | any orphan | any lag spike) +
     failover transition time (if injected). Exit 0.
   - **Exit codes**: 0 (clean 24h) / 1 (any threshold
     triggered) / 2 (bad flags) / 3 (bootstrap failed at
     T+5min).

5. **`docs/best-practices/quant/soak-results-template.md`** —
   human-fillable markdown table:
   - Per-hour rows: hour | elapsed | influx_klines |
     mongo_trades | node1_role | node2_role | node3_role
     | any_orphan | any_lag_spike
   - Failover section: kill_at | recovered_at |
     transition_time_sec | post_recovery_observations
   - Summary section: total_influx_rows | total_mongo_rows
     | total_decisions | total_errors | total_uptime_pct
   - Verdict section: PASS / FAIL with reasons
   - Sign-off section: human name, date, links to raw
     ticks (`bee --connect $LEADER kv list
     soak/run_$RUN_ID/`)

### Out of scope (deferred)

- File-backed KV (S33.1 Resolution #2, 1.x) — accepted
  that kill = fresh KV + lost local ticks; the failover
  injection's surviving node re-registers Datasources.
- Cross-host clusters, TLS, mDNS / DNS peer discovery
  (1.x).
- Real money / P&L — the demo runs on paper or sandbox
  Binance keys.
- Auto-rollback if the soak fails — the human decides;
  the S33 sign-off form captures the verdict.
- 24h wall-clock run itself — the agent ships the
  procedure, the human runs it.
- Refactor of `demo-quant-prod.sh` to share datasource +
  deploy code with `soak-quant-24h.sh` — the 24h loop
  duplicates the relevant lines (~40 lines) for
  readability; a follow-up can DRY it.

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│ scripts/soak-quant-24h.sh (single bash script)            │
│                                                            │
│  Phase 0: setup  →  Phase 1: cluster  →  Phase 2: leader  │
│  Phase 3: ds    →  Phase 4: deploy   →  Phase 5: bootstrap│
│  Phase 6: human →  Phase 7: 24h loop →  Phase 8: failover │
│  Phase 9: summary                                       │
└────┬────────────────────────────────────┬─────────────────┘
     │                                    │
     │  HTTP/curl                         │  admin RPC
     │  (InfluxDB Flux                    │  (bee --connect)
     │   + MongoDB                        │
     │   mongosh)                         │
     ▼                                    ▼
┌──────────────┐                  ┌─────────────────────┐
│ InfluxDB v2  │                  │ 3× bee node        │
│ (real)       │                  │ (TcpTransport)     │
│              │                  │  ├─ raft 7701-3    │
│ bucket:      │                  │  ├─ admin 8701-3   │
│  trading     │                  │  ├─ Raft KV        │
│              │                  │  ├─ HandlerVtable  │
│              │                  │  │  + report_stats │
│              │                  │  └─ mock plugins   │
└──────────────┘                  └─────────┬──────────┘
                                           │
                                           │ writes
                                           ▼
                                 ┌─────────────────────┐
                                 │ Leader's Raft KV    │
                                 │ key:                │
                                 │  soak/run_<id>/     │
                                 │   tick_<ts>         │
                                 │ value:              │
                                 │  bincode(TickMetrics)│
                                 └─────────────────────┘
                                           ▲
                                           │ kv list
                                           │ (post-soak)
                                           │
                                 ┌─────────┴──────────┐
                                 │ human fills        │
                                 │ soak-results-      │
                                 │ template.md        │
                                 └────────────────────┘
```

## Data flow

**Per-tick (every 5 min, default; 5s for --smoke)**:

1. `cluster status` → 3 roles + 3 log_lags
2. `jobs list` → N jobs × M tasks
3. `diagnostics <task_id>` (per task) → TaskDiagDetail with
   `runtime_stats`
4. InfluxDB Flux: `from(bucket:"trading") |> range(-5m) |>
   filter(_measurement=="klines") |> count()`
5. MongoDB aggregation: `db.trades.aggregate([
   {$match: {ts: {$gte: now-5min}}}, {$count: "n"}])`
6. Threshold check (5 conditions, see Phase 7 above)
7. `kv put soak/run_<RUN_ID>/tick_<TS_MS>
   <bincode(TickMetrics)>` (Raft commit, replicated to all
   3 nodes)

**Tick size estimate**: 8 fields × 100 bytes = ~800 bytes
per tick. 288 ticks = 230 KB total. Trivial for Raft.

**Post-soak**:

8. Human runs `bee --connect $LEADER kv list
   soak/run_<RUN_ID>/` to enumerate all 288 ticks
9. Human runs `jq` / `awk` / Excel on the bincode-decoded
   values
10. Human fills `soak-results-template.md`

## Wire format (new)

### `bincode(TickMetrics)`

```rust
pub struct TickMetrics {
    pub ts_unix_ms: u64,
    pub elapsed_sec: u64,
    pub cluster: ClusterMetricsDetail,   // already exists (S33.1)
    pub jobs: Vec<JobSummary>,            // already exists (S33.1)
    pub tasks: Vec<TaskDiagDetail>,      // extended (S33.2)
    pub influx_klines_per_min: u64,      // S33.2
    pub mongo_trades_per_min: u64,       // S33.2
    pub failover_at_ms: Option<u64>,     // S33.2
    pub recovered_at_ms: Option<u64>,    // S33.2
}
```

`bincode` (not JSON) for compactness and parity with the
existing S33.1 admin RPC. ~800 bytes/tick vs ~3KB JSON.

### `AdminRequest::ListKv { prefix: String }`

```rust
AdminRequest::ListKv { prefix: String }
```

### `AdminResponse::KvList(Vec<(String, Vec<u8>)>)`

```rust
AdminResponse::KvList(Vec<(String, Vec<u8>)>)
```

`Vec<u8>` is the raw bincode body; the CLI's
`bee --connect <addr> kv list <prefix>` decodes per known
schema (e.g. `soak/run_*/tick_*` → TickMetrics) or prints
`(key, value_len)` only.

## Error handling

| Failure mode | Behavior |
|--------------|----------|
| Bootstrap (T+5min) finds 0 influx klines | Exit 3 with "deploy did not produce klines within 5 min" |
| Bootstrap finds 0 mongo trades | Same; exit 3 |
| Any tick finds log_lag > 1000 | Exit 1 with the offending node_id |
| Any tick finds a task Orphaned for > 60s | Exit 1 with the task_id |
| influx_klines_per_min == 0 for 10 consecutive ticks (50 min) | Exit 1 (the loop maintains a rolling "0 rate" counter per sink) |
| mongo_trades_per_min == 0 for 10 consecutive ticks | Exit 1 |
| InfluxDB / MongoDB unreachable on a tick | Log "query failed", skip threshold check for that tick, continue. After 3 consecutive failures, exit 1 (the external sink is down). |
| A node's admin RPC is unreachable (failover) | The loop re-runs Phase 2 (leader discovery) every 5s until a new leader is found, then continues. Records `failover_at_ms` on the first failure; `recovered_at_ms` when new leader responds. |
| Bee itself crashes (kill -9 all 3) | The script's `trap EXIT` cleans up. Run is aborted, partial ticks lost. |
| Human Ctrl-C during 24h loop | `trap` cleans up, prints "Aborted at T+Xh Ym, partial results in /tmp/bee_soak/.", exit 130. |

## Testing strategy

- **Unit**: `KVStateMachine::list(prefix)` — empty / no-match /
  match / prefix-is-full-key. ~6 cases.
- **Unit**: `Op::List` + `apply_op` — same coverage.
- **Unit**: `AdminServer::dispatch(ListKv)` — auth (MVP: no
  auth), bincode round-trip, prefix filter, empty result.
- **Integration**: `kv_list_integration.rs` — boots a 3-node
  in-memory cluster, writes 3 keys with 2 different
  prefixes, asserts `List("prefix-a/")` returns 2 and
  `List("nope/")` returns 0.
- **Smoke**: `bash scripts/soak-quant-24h.sh --smoke` — 5
  min / 5s interval, asserts the loop runs end-to-end, ticks
  land in KV, and the summary table prints. ~5 min.
- **End-to-end**: human runs `bash
  scripts/soak-quant-24h.sh` for 24h with real credentials.
  This is the only way to verify the InfluxDB / MongoDB
  rate queries; agent cannot drive.

## Acceptance criteria

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` green (463 + new tests
      expected: ~6 KV list + ~3 admin RPC list + ~2
      TaskRuntimeStats)
- [ ] `bee node` starts an AdminServer on
      `<raft_port> + 1000` (default) or `--admin-bind`
- [ ] `bee --connect <addr> jobs list` returns the
      live-cluster's JobSummary (works end-to-end against
      `bee node` workers)
- [ ] `bee --connect <addr> diagnostics <id>` returns
      `TaskDiagDetail` with `runtime_stats` populated for
      tasks that have run ≥ 1 message
- [ ] `bee --connect <addr> kv list <prefix>` returns
      `(key, value)` pairs filtered by prefix
- [ ] `bash scripts/soak-quant-24h.sh --smoke` runs to
      completion in 5 min, prints summary, ticks land in
      KV, exit 0
- [ ] `bash scripts/soak-quant-24h.sh --smoke
      --failover-midway` triggers the kill at T+2.5min
      (half of 5min), records failover transition time
- [ ] `docs/best-practices/quant/soak-results-template.md`
      exists with fields for: per-hour InfluxDB /
      MongoDB counts, per-node uptime, failover
      transition time, total decisions, total errors
- [ ] A real human runs the 24h soak with real credentials,
      fills the template, and decides Y/N on the S33
      sign-off form's 3 remaining rows

## Out of scope (1.x concerns)

- File-backed KV (S33.1 Resolution #2, 1.x)
- Cross-host clusters, TLS, mDNS / DNS-based peer discovery
- Real money / P&L
- Auto-rollback on failure
- 24h wall-clock run (the human's, not the agent's)
- DRY refactor of `demo-quant-prod.sh` + `soak-quant-24h.sh`
- Production plugin instrumentation (S34–S39 each plugin's
  S33.2 follow-up)

## Resolutions (from brainstorming, 2026-06-10)

1. **S33.2 includes AdminServer-into-run_node** (S33.1
   follow-up, required for the 24h loop to read cluster
   state via `--connect`).
2. **TaskRuntimeStats is added as a new TaskDiagDetail
   field** with 6 sub-fields; data via HandlerVtable
   `report_stats` hook.
3. **Threshold values** adopted from the S33.2 story
   verbatim: log_lag > 1000 / Orphaned > 60s / sink rate
   == 0 for 10 min.
4. **Storage is Bee Raft KV via a new `Op::List` +
   `AdminRequest::ListKv`** — not CSV, not external
   InfluxDB. Loop writes 8 fields per tick; 288 ticks =
   230 KB total. Human reads back via
   `bee --connect $LEADER kv list soak/run_$RUN_ID/`.
5. **No file-backed KV** (S33.1 Resolution #2, 1.x).
   Failover injection accepts that a killed node's local
   state is lost; the surviving node re-registers
   Datasources + re-deploys pipelines during the
   failover transition.
6. **Smoke test runs 5 min** with 5s interval
   (`--smoke` flag, not the default 5 min interval).
7. **Human touchpoints**: T+0 (start), T+5min (Enter to
   confirm), T+24h+5min (review + fill template). 24h
   loop is autonomous. `--failover-midway` injects at
   T+12h, no human action needed.
8. **Approach: minimal** (per "Approach A" in
   brainstorming). Datasource + deploy steps duplicate
   `demo-quant-prod.sh` lines for readability; refactor
   is a follow-up.

## Open questions (none)

S33.2 has no remaining open questions. The 6 design
clarifications and the 2 follow-up questions were resolved
during brainstorming on 2026-06-10.
