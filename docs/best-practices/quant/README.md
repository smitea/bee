# Bee · Best Practices · Quant Trading

This section is the **quant trading reference implementation** for
Bee — a large chunk of real-world business example that exercises
every Bee feature end-to-end (Datasource management, Producer
sharing, plugin loading, FFI dispatch, SQL pipelines, deployment).

## What's here

- `stories.md` — the quant-trading implementation stories
  (S33 HITL milestone + S34–S40 production plugins + e2e
  deploy). Cross-references to other stories point back to
  the main repo's `docs/stories.md`.
- `adr/0011-stream-identity-and-backfill.md` — the
  Stream-identity ADR. Quant-specific (covers the Binance WS
  backfill-on-subscribe semantics).
- `examples/quant_btc_strategy.sql` + `quant_btc_strategy_backfill.sql`
  + `quant_btc_strategy_v2.sql` — the three production SQL pipelines
  (S40). The canonical one, the backfill warmup variant (with
  `from => '2024-06-01'`), and a second strategy on the SAME binance
  Stream (demonstrates Producer sharing).
- `scripts/demo-quant-prod.sh` — one-click end-to-end runner for the
  6 production plugins (S34–S39) + 3 SQL pipelines. **Requires
  real credentials in `scripts/.env`** (see `scripts/.env.example`).
  The earlier S33-deferred `docs/best-practices/quant/scripts/demo-quant-prod.sh`
  was an architecture-level smoke demo for the FFI + 5 mock plugins;
  the new `scripts/demo-quant-prod.sh` is the S40 e2e demo against
  real Binance WS / NewsAPI / InfluxDB v2 / MongoDB.
- `scripts/start-cluster.sh` + `scripts/kill-node.sh` — S33.1's
  multi-node + failover plumbing. Spawns 3 `bee node` worker
  processes on `127.0.0.1:7701..7703`; SIGKILLs one to demonstrate
  the production failure model ("the box dies"). Used by the S40
  demo's failover step when `BEE_MULTINODE=1` is set (off by
  default so the existing 23/23 `BEE_DRY_RUN=1` path stays green).
  The re-election itself is asserted by
  `crates/bee-control/src/raft/cluster_tcp_integration.rs`
  ::`tcp_3_node_survives_simulated_crash` (the production script
  verifies the OS-level kill + surviving-nodes-up contract).
- `scripts/soak-quant-24h.sh` — S33.2's 24h live-soak
  monitoring loop. Starts a 3-node cluster via
  `scripts/start-cluster.sh`, registers 4 Datasources,
  deploys 3 SQL pipelines, and runs a per-5-min tick
  loop (5s for `--smoke`) for 24h. Each tick writes
  JSON to `/tmp/bee_soak/<RUN_ID>_tick_<TS>.json`.
  Thresholds (any of which exit non-zero): log_lag >
  1000, task Orphaned > 60s, InfluxDB / MongoDB rate
  == 0 for ≥ 10 min. `--failover-midway` injects a
  SIGKILL at the T+12h mark. Used by the S33
  sign-off's 3 production-deployment rows (real
  money signals, InfluxDB data, MongoDB data).
  See `docs/best-practices/quant/soak-results-template.md`
  for the human-fillable results table.
- `specs/2026-06-08-s33-deferred-ffi-design.md` — the design
  spec for the FFI wire format + runtime plugin dispatching.
- `plans/2026-06-08-s33-deferred-ffi.md` — the implementation
  plan for the FFI + runtime dispatching.

## Why a separate section

The main repo's primary story is the **generic, domain-agnostic
Bee** — Producer sharing, plugin FFI, performance showcase (S41).
The quant trading example is too large and too domain-specific to
be the primary narrative; it's preserved here as a reference for
users who want to build real quant strategies on top of Bee.

The 5 plugins under `plugins/quant/` are *reference
implementations* — their plugin STRUCTURE is production-grade
(cdylib + FFI vtable + bincode wire format), but the data
sources (Binance WS, NewsAPI, InfluxDB v2, MongoDB, yata/ta-lib)
are placeholders. S34–S40 in `stories.md` replaced them with
real implementations (S34 binance, S35 google-news, S36 influxdb,
S37 mongodb, S38 ta-indicators, S39 onnx-ml).

## Running the e2e demo (S40)

The full e2e demo runs the 6 production plugins end-to-end against
real external services:

```bash
# 1. Supply credentials
cp scripts/.env.example scripts/.env
$EDITOR scripts/.env  # fill in NEWSAPI_KEY, INFLUXDB_TOKEN, INFLUXDB_ORG

# 2. Start local InfluxDB v2 + MongoDB (or set remote URLs in .env)

# 3. Run the demo
scripts/demo-quant-prod.sh
```

The script:

1. Builds all 6 production plugins in release mode.
2. Stages their cdylibs into `$BEE_PLUGIN_DIR` (default `/tmp/bee_prod_plugins`).
3. Starts a Bee node (single-node MVP; the 3-node cluster + failover
   demo is a 1.x feature).
4. Registers 4 Datasources (`binance`, `google_news`, `influxdb`,
   `mongodb`) with **connection-level** config only — `symbol`,
   `interval`, `measurement`, `collection` are per-call args in the
   SQL, not in the Datasource config (ADR-0010).
5. Deploys the 3 SQL pipelines in order:
   - `quant_btc_strategy_backfill.sql` — warm up state from 2024-06-01
   - `quant_btc_strategy.sql`        — the canonical strategy
   - `quant_btc_strategy_v2.sql`     — a second strategy on the SAME
     `binance` Stream (Producer sharing: exactly 1 binance Producer)
6. Verifies the InfluxDB `klines` measurement and the MongoDB
   `trades` collection both receive data.

To skip plugin rebuilds and go straight to deploy / verify:

```bash
scripts/demo-quant-prod.sh
# (the script always rebuilds; if you want a quick check of the
# rest of the flow without a full release build, set BEE_DRY_RUN=1)
```

To check what the script would do without touching the cluster
(skip the build, skip the `bee deploy`, skip the InfluxDB / MongoDB
queries — just verify the wiring):

```bash
BEE_DRY_RUN=1 scripts/demo-quant-prod.sh
```

## See also

- Main repo `docs/stories.md` for the generic Bee story set
  (S0–S31, S41).
- Main repo `docs/adr/0001`–`0010` for the generic architecture
  decisions.
- Main repo `plugins/` for the S41 performance plugins (land in
  a future session).
