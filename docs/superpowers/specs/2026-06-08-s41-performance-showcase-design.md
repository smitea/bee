# S41 · Performance showcase (5-minute demo) — 1-Node MVP Design

**Date**: 2026-06-08
**Status**: design — pending approval
**Owner**: docs + plugins + SQL runtime (one-shot)
**Story**: S41 (single S41 implementation; the N-node scaling part of the S41 spec is deferred to a follow-up session)
**Source spec**: [docs/stories.md §S41](docs/stories.md) (the canonical 270-line S41 spec). This design is the **1-node, in-process** adaptation of that spec.

## Context

The restructure (commit `b8c859f`) made S41 the new primary demo of the main repo. The current state has:
- 354 workspace tests passing
- 5 quant-flavored reference plugin crates at `plugins/quant/`
- DataFusion v49.0.2 as the SQL engine (no ASOF JOIN support in v49)
- `bee-plugin-sdk` with `BeeHostV1` (4 register-* function pointers; **no KV access**)
- `bee run <sql_file>` CLI (S26) for single-process SQL execution

The S41 spec in `docs/stories.md` (270 lines) describes a multi-node, cluster-scaled demo. This design adapts that spec to a **single-node, in-process** execution mode. The N-node scaling (3/5-node cluster, `bee deploy`, `bee jobs wait`, cluster scripts) is **deferred** to a follow-up session and remains in the S41 spec for that future work.

The user has chosen to ship the **full feature set** of the 3 demos in 1-node mode:
- Fibonacci (stateful UDF + KV state)
- Prime sieve (multi-Phase SQL + hard correctness check `n_primes = 5761455`)
- Multi-stream analytics (3 test-fixture streams + ASOF JOIN + WINDOW TUMBLING + multi-sink)

This requires:
- **DataFusion v49 → v50+** (v50 introduced ASOF JOIN; latest stable at design time)
- **BeeHostV1 extension** with `kv_get` / `kv_put` / `kv_cas` function pointers (for `fib_step` state)
- **`console` sink** built into bee-dsl-sql (no external sink needed)
- **2 test fixtures** in bee-dsl-sql (feature-gated): `generate_series`, `generate_events`
- **1 new plugin**: `plugins/bee-plugin-perf-fib/` (Fibonacci UDFs, KV-backed state)
- **3 SQL pipelines** under `examples/performance/`
- **1 demo script** (`scripts/demo-perf.sh`) that runs all 3 + prints a measured perf table
- **3 doc updates** (README, product-design §4, `examples/performance/README.md`)

## Architecture (the new components)

```
/Users/shaw/Developer/rust/bee/
├── Cargo.toml                                  (UPDATE: datafusion "49" → ">=50",<52)
├── crates/
│   ├── bee-dsl-sql/
│   │   ├── Cargo.toml                          (UPDATE: add `test-fixtures` feature)
│   │   └── src/
│   │       ├── lib.rs                          (UPDATE: register console sink, fixtures)
│   │       ├── sinks/console.rs                (NEW: console sink implementation)
│   │       └── test_fixtures.rs                (NEW: generate_series + generate_events, cfg-gated)
│   └── bee-plugin-sdk/
│       └── src/lib.rs                          (UPDATE: add kv_get / kv_put / kv_cas to BeeHostV1)
├── bee/src/main.rs                             (UPDATE: register kv_* host-side wiring for plugins)
├── plugins/
│   ├── quant/                                  (unchanged; the 5 quant plugins)
│   └── bee-plugin-perf-fib/                    (NEW: the only domain-specific plugin for the demo)
│       ├── Cargo.toml
│       ├── src/lib.rs                          (fib_step + fib_seed; KV-backed state)
│       ├── tests/state.rs                      (unit tests, incl. restart-survives)
│       └── README.md
├── examples/performance/                       (NEW directory: 3 SQL demos)
│   ├── fibonacci.sql
│   ├── prime_sieve.sql
│   ├── multi_stream_analytics.sql
│   └── README.md
├── scripts/demo-perf.sh                        (NEW: 1-click demo script)
├── README.md                                   (UPDATE: "Performance Demos" section)
└── docs/product-design.md                      (UPDATE: §4 already mentions S41; expand demo links)
```

## Components in detail

### 1. `plugins/bee-plugin-perf-fib/`

**Purpose**: the only domain-specific code in the S41 demo.

**Files**:
- `Cargo.toml`: `crate-type = ["cdylib"]`; deps: `bee-plugin-sdk`, `tokio`, `serde`, `bincode`, `once_cell`
- `src/lib.rs`:
  - `pub fn fib_step(n: u64) -> i128` — stateful; calls `host.kv_get(state_key)`, parses `(prev2, prev1)`, computes `prev2 + prev1`, calls `host.kv_put(state_key, ...)` to update state, returns the new value
  - `pub fn fib_seed(n: u64) -> i128` — stateless; returns 0 if n == 0, 1 otherwise
  - `pub fn plugin_manifest() -> PluginManifest` — declares 2 Handler descriptors: `fib_step`, `fib_seed`
  - `cdylib_plugin!(Factory)` — the FFI entry point
- `tests/state.rs`:
  - `test_fib_seed_correctness` — first 20 values
  - `test_fib_step_state_round_trip` — compute 100 values, "restart" (re-instantiate state), verify 101st value is correct
- `README.md` — UDF docs + state key format

**State key**: `state/handler/<stream_id>/fib_step/state` (bincode-serialized `(i128, i128)`)

**Stream ID**: passed via the SQL call's first argument (the `n` parameter) — the plugin derives the stream_id from a `bee_plugin_sdk::StreamId` parameter (NEW addition, see BeeHostV1 extension below).

### 2. BeeHostV1 KV extension

**Current BeeHostV1** (in `crates/bee-plugin-sdk/src/lib.rs`):
```rust
pub struct BeeHostV1 {
    pub ctx: *mut c_void,
    pub register_adapter: Option<...>,
    pub register_input_adapter_vtable: Option<...>,
    pub register_output_adapter_vtable: Option<...>,
    pub register_handler_vtable: Option<...>,
}
```

**Extended BeeHostV1** (add 4 function pointers):
```rust
pub struct BeeHostV1 {
    // ... existing 4 fields unchanged ...
    pub kv_get: Option<unsafe extern "C" fn(
        ctx: *mut c_void,
        key: *const c_char,
        out_value: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32>,  // 0 = found, 1 = not found, -1 = error
    pub kv_put: Option<unsafe extern "C" fn(
        ctx: *mut c_void,
        key: *const c_char,
        value: *const u8,
        len: usize,
    ) -> i32>,  // 0 = ok, -1 = error
    pub kv_cas: Option<unsafe extern "C" fn(
        ctx: *mut c_void,
        key: *const c_char,
        expected: *const u8,
        exp_len: usize,
        new: *const u8,
        new_len: usize,
    ) -> i32>,  // 0 = ok, 1 = mismatch, -1 = error
    pub current_stream_id: Option<unsafe extern "C" fn(
        ctx: *mut c_void,
        out_id: *mut [u8; 32],
    ) -> i32>,  // 0 = ok, -1 = error
}
```

**Host-side wiring** (in `bee/src/main.rs` `run_plugin_cli` or wherever plugins are loaded):
- `kv_get` / `kv_put` / `kv_cas` → in-process `bee-kv-test` store (which is a HashMap-backed test impl of the KV API; the production impl is Raft-replicated)
- `current_stream_id` → returns a stable hash of the SQL call's call-site (so the same call gets the same stream_id across plugin restarts)

**Safe Rust wrapper** (in bee-plugin-sdk):
```rust
impl BeeHostV1 {
    pub fn safe_kv_get(&self, key: &str) -> Result<Option<Vec<u8>>, SdkError> { ... }
    pub fn safe_kv_put(&self, key: &str, value: &[u8]) -> Result<(), SdkError> { ... }
    pub fn safe_kv_cas(&self, key: &str, expected: &[u8], new: &[u8]) -> Result<bool, SdkError> { ... }
    pub fn safe_current_stream_id(&self) -> Result<[u8; 32], SdkError> { ... }
}
```

The plugin uses the safe wrappers, not the raw FFI pointers.

### 3. `crates/bee-dsl-sql/src/test_fixtures.rs` (feature-gated)

**Feature**: add `test-fixtures` to `bee-dsl-sql`'s Cargo.toml. The test fixture module is gated behind `#[cfg(feature = "test-fixtures")]`. Production builds don't include the feature, so the functions are not in the production binary.

**Functions**:
- `generate_series(start: i64, end: i64) -> Stream<i64>` — emits one event per integer in `[start, end]`. Implemented as a `TableFunction` that returns a `RecordBatch` of one column.
- `generate_events(schema: StructType, count: u64, seed: u64) -> Stream<StructType>` — emits `count` deterministic pseudo-random events with the given schema. Implemented as a `TableFunction` that uses a deterministic LCG (linear congruential generator) seeded with `seed`.

**Registration**:
- `lib.rs` registers both when `feature = "test-fixtures"` is on
- `bee run` enables the feature by default (for the S41 demo)
- Production deployments of bee-dsl-sql do not enable the feature

### 4. `crates/bee-dsl-sql/src/sinks/console.rs` (always-on)

**Purpose**: built-in console sink so the S41 demos can emit results to stdout without a real external sink.

**Syntax** (in SQL):
```sql
EMIT INTO console SELECT ... FROM ...;
```

The `console` keyword is recognized by the `EMIT INTO` parser as a built-in sink. The console sink writes the resulting rows to stdout (one row per line, JSON-formatted).

**Files**:
- `src/sinks/console.rs` — the `Sink` trait impl that writes to stdout
- `src/lib.rs` — register the console sink in the `EMIT INTO` parser

### 5. DataFusion upgrade (49 → 50+)

**Risk**: DataFusion 50 may have breaking changes in the SQL parser, executor API, or session API. The 354 existing tests should catch most regressions.

**Migration steps**:
1. Bump `Cargo.toml` workspace `datafusion = "49"` → `datafusion = "50"` (or latest 50.x stable at design time)
2. Run `cargo build --workspace` — fix compile errors
3. Run `cargo test --workspace` — fix test failures
4. Verify ASOF JOIN works (write a small test that uses ASOF JOIN syntax)
5. Verify all existing SQL still parses + executes

If 50.x has too many breaking changes, fall back to 50.0.0 (the first 50.x release) and patch as needed.

### 6. Three SQL pipelines (`examples/performance/*.sql`)

#### `fibonacci.sql`

```sql
use perf_fib;

CREATE SOURCE naturals AS
SELECT n FROM generate_series(1, 1000000);

CREATE VIEW fib_stream AS
SELECT
    n,
    fib_step(n) AS fib_value
FROM naturals;

EMIT INTO console
SELECT n, fib_value FROM fib_stream WHERE n <= 20;
```

**State**: fib_step writes 2 values to KV per call (`(prev2, prev1)`). For 1M calls, that's 1M KV writes. The demo measures wall-clock of the 1M-call run.

#### `prime_sieve.sql`

```sql
CREATE SOURCE naturals AS
SELECT n FROM generate_series(2, 100000000);

CREATE VIEW sieved_2 AS
SELECT n FROM naturals WHERE n = 2 OR n % 2 != 0;
CREATE VIEW sieved_3 AS
SELECT n FROM sieved_2 WHERE n = 3 OR n % 3 != 0;
-- ... continues for primes 5, 7, 11, 13, ..., up to 997 (the first 20 primes after 2, 3)

CREATE VIEW prime_count AS
SELECT count(*) AS n_primes FROM sieved_997;

EMIT INTO console SELECT * FROM prime_count;
```

**Hard correctness check**: `n_primes = 5761455` (primes ≤ 10^8). The demo script asserts this and fails loudly if the count is wrong.

**Performance note**: for 1-node mode, all 20 `sieved_p` Phases are in-process, no cross-Node data channels. The wall-clock is the total time to sieve 10^8 integers through 20 filters.

#### `multi_stream_analytics.sql`

```sql
CREATE SOURCE clicks AS
SELECT user_id, ts, page FROM generate_events(
    struct_pack(user_id INT, ts TIMESTAMP, page STRING),
    100000, seed => 42
);

CREATE SOURCE views AS
SELECT user_id, ts, duration_ms INT FROM generate_events(
    struct_pack(user_id INT, ts TIMESTAMP, duration_ms INT),
    50000, seed => 43
);

CREATE SOURCE purchases AS
SELECT user_id, ts, amount DECIMAL FROM generate_events(
    struct_pack(user_id INT, ts TIMESTAMP, amount DECIMAL),
    10000, seed => 44
);

CREATE VIEW per_minute AS
SELECT
    window_start(c.ts, INTERVAL '1' MINUTE) AS minute,
    count(DISTINCT c.user_id) AS unique_clickers,
    count(DISTINCT p.user_id) AS unique_buyers,
    sum(p.amount) AS revenue
FROM clicks c
LEFT ASOF JOIN views v ON c.user_id = v.user_id AND c.ts >= v.ts
LEFT ASOF JOIN purchases p ON c.user_id = p.user_id AND c.ts >= v.ts
WINDOW TUMBLING (c.ts, INTERVAL '1' MINUTE)
GROUP BY minute;

EMIT INTO console
SELECT * FROM per_minute ORDER BY minute LIMIT 60;
```

**DataFusion v50 dependencies**:
- `ASOF JOIN` — v50 native support
- `WINDOW TUMBLING` — DataFusion has window functions natively; the SQL syntax may need a custom parser extension (verify in v50)
- `struct_pack(...)` — DataFusion 50 has `struct()` constructor; check exact syntax
- `window_start(c.ts, INTERVAL '1' MINUTE)` — DataFusion has `date_trunc` natively; may need a SQL alias

If any of these require custom SQL extensions, add them to `bee-dsl-sql/src/parser.rs` (custom Statement variant) per the S29 pattern.

### 7. `scripts/demo-perf.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Build the perf-fib plugin
(cd plugins/bee-plugin-perf-fib && cargo build --quiet)

# Demo 1: Fibonacci (1M values)
echo "==== Fibonacci (1M values) ===="
T0=$(date +%s%N)
cargo run -q -p bee --bin bee -- run examples/performance/fibonacci.sql
T1=$(date +%s%N)
FIB_MS=$(( (T1 - T0) / 1_000_000 ))
FIB_TPUT=$(( 1_000_000 * 1_000_000_000 / (T1 - T0) ))

# Demo 2: prime sieve (≤ 10^8, 20 primes)
echo "==== Prime sieve (≤ 10^8) ===="
T0=$(date +%s%N)
cargo run -q -p bee --bin bee -- run examples/performance/prime_sieve.sql
T1=$(date +%s%N)
SIEVE_MS=$(( (T1 - T0) / 1_000_000 ))

# Hard correctness check: sieve must produce 5761455 primes ≤ 10^8
N=$(cargo run -q -p bee --bin bee -- run examples/performance/prime_sieve.sql 2>&1 | grep -oE 'n_primes=[0-9]+' | tail -1 | cut -d= -f2)
if [ "$N" -ne 5761455 ]; then
    echo "FAIL: prime count mismatch (expected 5761455, got $N)"
    exit 1
fi

# Demo 3: multi-stream analytics
echo "==== Multi-stream analytics (160K events) ===="
T0=$(date +%s%N)
cargo run -q -p bee --bin bee -- run examples/performance/multi_stream_analytics.sql
T1=$(date +%s%N)
MS_MS=$(( (T1 - T0) / 1_000_000 ))
MS_TPUT=$(( 160_000 * 1_000_000_000 / (T1 - T0) ))

# Print measured perf table
cat <<EOF

==== Measured performance (1 Node) ====
| Demo                      | Wall-clock   | Throughput             |
|---------------------------|--------------|------------------------|
| Fibonacci (1M values)     | ${FIB_MS} ms | ${FIB_TPUT} events/sec |
| Prime sieve (≤ 10^8)      | ${SIEVE_MS} ms| (N/A, 10^8 ints)       |
| Multi-stream analytics    | ${MS_MS} ms  | ${MS_TPUT} events/sec  |
EOF
```

**Note**: the script runs `cargo run` for each demo. This is slow (~30s startup per demo). For a true perf measurement, the binary should be pre-built and run directly. Future enhancement: pre-build once, then time the actual demo runs only.

### 8. Docs

#### `examples/performance/README.md`

Explains:
- The 3 demos and what each measures
- The math (Fibonacci recurrence, prime sieve correctness, multi-stream aggregation)
- How to read the perf table
- How to extend the demos (e.g., add more primes, change the window size)

#### `README.md` update

Add a "Performance Demos" section near the bottom (before the Quant trading reference section):
```markdown
## Performance Demos

The performance showcase (`scripts/demo-perf.sh`) is the new
primary 5-minute demo of the main repo. It runs 3 demo pipelines
end-to-end and prints a measured performance table:

- **Fibonacci**: 1M values via stateful `fib_step` UDF + KV-backed state
- **Prime sieve**: 10^8 integers via 20 sequential sieving Phases (correctness: 5,761,455 primes)
- **Multi-stream analytics**: 160K events across 3 streams with ASOF JOIN + WINDOW TUMBLING

See [`examples/performance/README.md`](examples/performance/README.md) for the math and Bee design choices.
```

#### `docs/product-design.md` update

The §4.1 "Performance showcase" already mentions S41. Add a sentence pointing to the new `examples/performance/README.md` and confirm the demo is now runnable (no longer "in flight").

## Implementation order (for the plan)

1. **DataFusion upgrade** (49 → 50+) — fix all regressions, verify 354 tests still pass
2. **BeeHostV1 KV extension** — add 4 FFI function pointers, host-side wiring, safe Rust wrappers, test
3. **`bee-plugin-perf-fib`** — new plugin, fib_step + fib_seed, KV-backed state, unit tests including restart-survives
4. **Test fixtures** (`generate_series`, `generate_events`) in bee-dsl-sql, feature-gated
5. **Console sink** — `EMIT INTO console` support in bee-dsl-sql
6. **3 SQL pipelines** — fibonacci, prime_sieve, multi_stream_analytics
7. **`scripts/demo-perf.sh`** — runs all 3, prints perf table, hard correctness check
8. **Docs** — examples/performance/README.md, README update, product-design update
9. **Final verification** — script runs all 3 demos in < 5 min, hard correctness check passes, 354 tests still pass

## Risks

1. **DataFusion 50 breaking changes** — could be 1-2 hours of debugging to fix compile/test errors. Mitigation: pin to 50.0.0 (the first 50.x) as a stable baseline.
2. **WINDOW TUMBLING / struct_pack syntax** — DataFusion 50 may not support the exact SQL syntax in the S41 spec. Mitigation: extend the bee-dsl-sql parser with custom Statement variants (S29 pattern).
3. **KV FFI surface evolution** — the 4 function pointers are MVP; a future S-XX story may need a richer KV API. The MVP is intentionally minimal.
4. **`cargo run` startup cost in perf measurement** — each `cargo run` invocation has ~30s startup overhead, which dominates the actual demo runtime. The first version of the script accepts this; a future enhancement would pre-build the binary.
5. **Prime sieve runtime** — sieving 10^8 integers through 20 filters on a single Node may take several minutes. The S41 spec's "5-minute demo" budget may be tight. Mitigation: reduce to 10 primes (sieves up to 29) for the demo; the spec allows this.
6. **Multi-stream analytics with ASOF JOIN on 160K events** — DataFusion 50's ASOF JOIN may have performance limitations. The first version of the demo accepts whatever perf DataFusion produces; a future enhancement would profile + optimize.

## Acceptance criteria

- [ ] DataFusion upgraded to v50+; all 354 existing tests still pass
- [ ] `BeeHostV1` has 4 new function pointers: `kv_get`, `kv_put`, `kv_cas`, `current_stream_id`
- [ ] `bee-plugin-perf-fib` is a workspace member; `Cargo.toml` declares `crate-type = ["cdylib"]`
- [ ] `fib_step` is correct against the first 20 Fibonacci values
- [ ] `fib_step` state round-trip: compute 100 values, simulate plugin restart, verify 101st value is correct
- [ ] `generate_series` and `generate_events` are gated behind `#[cfg(feature = "test-fixtures")]`; production build does not include them
- [ ] `EMIT INTO console` works (writes rows to stdout)
- [ ] `examples/performance/fibonacci.sql` compiles and emits the first 20 fib values in the correct order
- [ ] `examples/performance/prime_sieve.sql` compiles and the console emits `n_primes = 5761455` (hard correctness check)
- [ ] `examples/performance/multi_stream_analytics.sql` compiles and emits a non-empty per-minute aggregation
- [ ] `scripts/demo-perf.sh` runs all 3 demos on a single Node and prints a measured performance table
- [ ] `README.md` "Performance Demos" section links to `scripts/demo-perf.sh` and `examples/performance/README.md`
- [ ] `docs/product-design.md` §4 references the runnable demo
- [ ] `examples/performance/README.md` explains the math, the Bee design, and how to read the numbers
- [ ] All 354 existing tests still pass after the S41 implementation

## Out of scope (deferred)

- N-node cluster scaling (3 / 5 Nodes) — S41 spec's "Performance table" with columns for 1/3/5 Nodes becomes "Performance table" with only the 1-Node column
- `bee deploy` and `bee jobs wait` CLI subcommands — S41 demo uses `bee run` (S26) for single-process execution
- Cluster scripts (`scripts/start-cluster.sh`, `scripts/load-plugin.sh`)
- Killing a Node mid-sieve to test Work-Stealing (1-node mode has nothing to kill)
- Production plugin examples in `plugins/` (S41 leaves the production-plugin directory empty for future S-XX stories)
- The 2 pre-existing warnings in `crates/bee-control/tests/{deploy_pipeline,raft_cluster}.rs`

## What stays in main

All of the above is in scope for this commit. The single commit will be on top of `b8c859f` (the restructure commit).
