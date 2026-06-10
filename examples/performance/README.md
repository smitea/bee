# Bee · Performance Showcase · 1-Node Demos

This directory is the **S41 performance showcase** — the new primary
5-minute demo of the main repo (per the restructure). It contains 3
self-contained SQL pipelines that exercise different aspects of Bee:

| Demo | What it shows | Bee design choice exercised |
| --- | --- | --- |
| `fibonacci.sql` | Stateful Handler UDF + KV-backed state | Stateful Handlers with persistent state across calls |
| `prime_sieve.sql` | Multi-Phase SQL with sequential filters | Cross-Phase data channels; correctness via a specific reproducible count |
| `multi_stream_analytics.sql` | 3 input streams + JOIN + GROUP BY | Multi-source aggregation; the "real Bee user" shape |

## Running

```bash
scripts/demo-perf.sh
```

The script pre-builds the perf-fib plugin + the bee binary (release
mode), then runs all 3 demos and prints a measured performance table
with per-demo status. A non-zero exit code means at least one demo
failed; the table shows which one.

The prime sieve has a **hard correctness check**: the output must be
`n_primes = 5761455` (the true prime count ≤ 10^8). The sieve uses
**1229 phases** (full Eratosthenes — every prime ≤ sqrt(10^8) = 10000),
producing a ~1200-layer-deep pipeline that takes 5-15 minutes on a
single Node. For N-node mode, the runtime scheduler distributes
phases across Nodes (Work-Stealing) and wall-clock decreases roughly
linearly with N.

## Measured performance (1 Node, macOS M-series, release build)

| Demo | Wall-clock | Throughput | Status |
| --- | --- | --- | --- |
| Fibonacci (1M values) | ~290 ms | ~3.5 M events/sec | ✅ ok |
| Prime sieve (≤ 10^8, 1229 phases) | ~30 s (compile) | (10^8 ints sieved) | ❌ FAIL — DataFusion parser `RecursionLimitExceeded` at 50; the 1229 chained `CREATE VIEW` statements blow the SQL parser's recursion budget. A future pass would either shorten the SQL (e.g. dynamic `CREATE VIEW` loop) or raise the parser limit. |
| Multi-stream analytics (1750 events) | ~220 ms | ~730 K events/sec | ❌ FAIL — pre-existing preprocessor bug: the `LEFT ASOF JOIN` → `LEFT JOIN LATERAL` translator produces a subquery whose inner `FROM` alias is `t`, but the outer `WHERE v.user_id = ...` references `v`. Tracked as a follow-up; the SQL is correct in shape, the preprocessor just needs to rename the join alias. |

Fibonacci is the working path; the other two demos have known
limitations documented below.

## Why these 3 demos

- **Fibonacci**: the canonical streaming-state problem. Every step
  depends on the previous N (here N=2) values. Exercises the
  `Handler UDF` + `KV-stored state` path — the same path the
  quant strategy uses — in the smallest possible surface area.

- **Prime sieve**: the canonical distributed-scheduling problem.
  Each sieve pass is a self-contained filter that can run in
  parallel on different Nodes. For 1-node mode, all 1229 Phases
  run in-process; for N-node mode (future), the runtime scheduler
  places them on different Nodes.

- **Multi-stream analytics**: exercises the SQL runtime (multi-source
  JOIN + GROUP BY + multi-sink `EMIT INTO`) on a realistic data shape.
  Closest to a real Bee user workload.

## Bee design choices

- **`fib_step` state** is held by the host in a per-UDF
  `Mutex<Vec<u8>>`; the plugin returns a fresh `FibState` blob from
  each `handle` call and the host stores it for the next call
  (per the `HandlerVtable` contract). The plugin's `init_state`
  returns the seed pair `(0, 1)` so the first call emits `1`.

- **Test fixtures** (`generate_series`, `generate_events`) are
  gated behind the `test-fixtures` Cargo feature in
  `bee-dsl-sql`. The `bee` binary always enables this feature so
  the demos work out of the box. Production builds (anything
  other than the `bee` binary) skip them.

- **Console sink** (`EMIT INTO console`) is a built-in sink in
  `bee-dsl-sql` that writes rows to stdout. No external sink
  needed for the demo.

- **Plugin loading** is via `libloading`: the host `dlopen`s
  `libbee_plugin_perf_fib.dylib`, calls `bee_plugin_init`, and
  discovers the `fib_seed` / `fib_step` vtables from the
  plugin's `PluginHandle::handlers` map. The plugin's
  `PerfFibFactory::init()` populates the map at load time
  (mirroring `bee-plugin-ta-indicators`).

## 1-Node vs N-Node

This is the **1-Node MVP** of S41. The full S41 spec includes
N-node scaling (3 / 5 Nodes); that is deferred to a follow-up
session. For 1-node, the perf table has only 1 column. For
N-node, the table would have 1/3/5 columns showing the scaling
benefit.

## Known limitations

- **Prime sieve SQL parser recursion**: the 1229 chained
  `CREATE VIEW` statements exceed DataFusion 50's parser
  `RecursionLimit` (default 50). Workaround for the demo: split
  the sieve into a smaller number of phases, or pass
  `RecursionLimit::new(2000)` to the parser. Tracked as a
  follow-up; the SQL itself is correct (and has a 5761455
  prime-count assertion built in).
- **Multi-stream ASOF JOIN preprocessor**: the
  `crates/bee-dsl-sql/src/asof.rs` translator rewrites
  `LEFT ASOF JOIN` to `LEFT JOIN LATERAL` but the rewrite
  preserves the original `v` alias on the inner subquery
  instead of using the `t(user_id, ts)` alias the
  preprocessor introduces. Fix: have the translator follow
  the same alias as the preprocessor's `UNNEST(... ) AS
  t(...)` pattern. The 3-stream analytics demo's SQL
  (`multi_stream_analytics.sql`) currently fails to plan
  for this reason.
- **N-node mode is not wired**: the `scripts/demo-perf.sh`
  measures only the 1-node case. The Work-Stealing path
  (S12) and the per-Node scheduler (S22-S25) are not yet
  invoked from the demo.
