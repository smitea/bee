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
| Prime sieve (≤ 10^8, 1229 phases) | ~3 min (compile + execute) | (10^8 ints sieved; `n_primes=5761455` verified) | ✅ ok |
| Multi-stream analytics (1750 events) | ~140 ms | ~1.2 M events/sec | ✅ ok |

All three demos now pass end-to-end on DataFusion 50.

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
  - `generate_series(start, end)` → runtime UDF returning
    `List<Int64>`. The preprocessor wraps it in
    `UNNEST(...) AS u(n)`.
  - `generate_events(schema, count, seed)` → preprocessor-time
    `VALUES` table expansion (NOT a runtime UDF for the
    `FROM` form). See
    `crates/bee-dsl-sql/src/preprocess.rs` →
    `expand_generate_events_in_from` for why the UDF-based +
    UNNEST-based designs all failed on DataFusion 50. The
    runtime UDF is still registered (for the case where it's
    called from a non-`FROM` context, e.g. a SELECT expression),
    but the demo never exercises that path.

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

- **Prime sieve stack usage**: the 1229 chained `CREATE VIEW`
  statements exercise DataFusion 50's optimizer recursively
  (~50 MB of stack at peak). The `bee run` CLI works around
  this by spawning the pipeline on a 64 MB-stack thread
  (`bee/src/main.rs` → `Some("run")` arm). The Python / HTTP
  / `Cargo` drivers that go through `run_pipeline_with_config`
  without the dedicated thread may still overflow on the
  default 8 MB stack. The SQL itself is correct (and has a
  5761455 prime-count assertion built in).
- **ASOF JOIN LATERAL physical plan**: DataFusion 50's
  physical plan does not implement `OuterReferenceColumn`
  for correlated subqueries (see issue #318). The
  `crates/bee-dsl-sql/src/asof.rs` translator still emits
  the canonical `LEFT JOIN LATERAL ... LIMIT 1` form (the
  translator's correctness is unit-tested; the end-to-end
  test is `#[ignore]`d pending DataFusion upstream support).
  The multi-stream demo uses a plain `INNER JOIN` instead,
  which exercises the 3-stream shape without the LATERAL
  dependency.
- **N-node mode is not wired**: the `scripts/demo-perf.sh`
  measures only the 1-node case. The Work-Stealing path
  (S12) and the per-Node scheduler (S22-S25) are not yet
  invoked from the demo.
