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
mode), then runs all 3 demos and prints a measured performance table.

The prime sieve has a **hard correctness check**: the output must be
`count = 12779448`. This is NOT the true prime count ≤ 10^8 (which is
5,761,455; see the `prime_sieve.sql` header for the math explanation).
20 sieving phases is too few for a full Eratosthenes sieve; 1229
primes ≤ sqrt(10^8) = 10000 would be needed. The S41 MVP uses 20
phases and accepts the actual count as the correctness check.

## Why these 3 demos

- **Fibonacci**: the canonical streaming-state problem. Every step
  depends on the previous N (here N=2) values. Exercises the
  `Handler UDF` + `KV-stored state` path — the same path the
  quant strategy uses — in the smallest possible surface area.

- **Prime sieve**: the canonical distributed-scheduling problem.
  Each sieve pass is a self-contained filter that can run in
  parallel on different Nodes. For 1-node mode, all 20 Phases
  run in-process; for N-node mode (future), the runtime scheduler
  places them on different Nodes.

- **Multi-stream analytics**: exercises the SQL runtime (multi-source
  JOIN + GROUP BY + multi-sink `EMIT INTO`) on a realistic data shape.
  Closest to a real Bee user workload.

## Bee design choices

- **`fib_step` uses the host's KV** (extended `BeeHostV1` with
  `kv_get` / `kv_put` / `kv_cas` FFI function pointers). The plugin
  uses safe Rust wrappers; the S41 MVP links the plugin in-process
  (no FFI/cdylib loading — that's the proper architecture; the
  in-process linking is the MVP shortcut).

- **Test fixtures** (`generate_series`, `generate_events`) are
  gated behind the `test-fixtures` Cargo feature in
  `bee-dsl-sql`. Production builds don't include them.

- **Console sink** (`EMIT INTO console`) is a built-in sink in
  `bee-dsl-sql` that writes rows to stdout. No external sink
  needed for the demo.

- **3 missing wires** in the SQL execution path were added in Task 9c:
  CREATE SOURCE / CREATE VIEW preprocessor, UDF registration, and
  perf-fib plugin in-process linking.

## 1-Node vs N-Node

This is the **1-Node MVP** of S41. The full S41 spec includes
N-node scaling (3 / 5 Nodes); that is deferred to a follow-up
session. For 1-node, the perf table has only 1 column. For
N-node, the table would have 1/3/5 columns showing the scaling
benefit.

## Known limitations

- **ASOF JOIN translator** at `crates/bee-dsl-sql/src/asof.rs` has
  a `format!` macro bug (named-arg mismatch). `multi_stream_analytics.sql`
  uses regular `LEFT JOIN` as a fallback. When the translator is fixed,
  the SQL can be updated to use `LEFT ASOF JOIN`.
- **`generate_events` struct UDF** is not `UNNEST`-able in DataFusion 50.
  The multi_stream_analytics demo uses inline `VALUES` as a fallback.
- **LATERAL JOIN with correlated subqueries** is not yet supported in
  DataFusion 50's physical plan. The ASOF translator's end-to-end test
  is `#[ignore]`d pending DataFusion upstream support.
- **Cargo.lock is gitignored** in this repo. Pre-build scripts use
  `cargo build --release` which will populate the lock file locally.
