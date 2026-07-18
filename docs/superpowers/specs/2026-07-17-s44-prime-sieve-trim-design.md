# S44 — S41 Demo Cleanup (prime_sieve trim + analytics simplify)

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S41 (performance showcase)
**ADRs:** none
**Status:** Draft (pending review)
**Source WIP:** `stash@{0}` — `examples/performance/{prime_sieve.sql, fibonacci.sql, multi_stream_analytics.sql}` + `scripts/demo-perf.sh`

## Why this story exists

The S41 demo's `prime_sieve.sql` ships with **1229 sieving phases** (every prime ≤ 10⁴). Running the full sieve to find primes ≤ 10⁸ takes ~3 minutes, which breaks the 5-minute evaluator walkthrough promised by the S41 spec. S44 trims the demo to a faster, demo-friendly shape.

S44 also simplifies `multi_stream_analytics.sql` (the stash removed a wall of explanatory comments that read more like design docs than demo code; the test logic is unchanged but the file is more readable).

## Stash WIP starting point

The stash contains four changes:

| File | Stash delta | Direction |
|---|---|---|
| `examples/performance/prime_sieve.sql` | 3707 → 73 lines | Trim from 1229 phases to **20 phases** covering primes 2..71. The new `prime_count` is the count of primes ≤ 71 = **20**. |
| `examples/performance/fibonacci.sql` | comments only | Removed a stale comment block that documented expected fib values (incorrect — `fib_step(1)` returns 0, not 1, per the FibState docstring). |
| `examples/performance/multi_stream_analytics.sql` | simplified comments + bumped event counts | Removed a long design-doc comment; bumped event counts (clicks 1000 → 100000, views 500 → 50000, purchases 250 → 10000); renamed `joined` → `per_minute`; schema now includes extra columns (`page`, `duration_ms`, `amount`) for the demo. |
| `scripts/demo-perf.sh` | modified | TBD — needs review against the new SQL content. |

## Scope

### In scope

1. **Apply the trimmed `prime_sieve.sql`** (the 20-phase version, primes 2..71). The expected `n_primes` becomes **20** (not 5,761,455).
2. **Apply the simplified `multi_stream_analytics.sql`** (stashes the cleanup).
3. **Apply the `scripts/demo-perf.sh` update** if the stash's version is correct; otherwise leave as-is and just add a note.
4. **Skip the `fibonacci.sql` comment cleanup** — low value, the stale comment is technically wrong but the demo runs correctly; cleanup can be a separate PR.
5. **Verify**:
   - `cargo test -p bee-dsl-sql` still green
   - `cargo build --workspace` still green
   - `bee run examples/performance/prime_sieve.sql` runs in < 5s and outputs `n_primes = 20`
   - `bee run examples/performance/multi_stream_analytics.sql` runs end-to-end and emits a non-empty per-minute aggregation

### Out of scope (deferred)

- **Restoring the full 10⁸ sieve** behind a `BEE_FULL_SIEVE=1` env var — the MVP trimmed version is sufficient for a 5-minute evaluator walkthrough. A follow-up story (S44.x) can add the env-var-controlled full version if evaluators want it.
- **Per-Pipeline timing in `scripts/demo-perf.sh`** — the script's measured table is currently a static template; the script doesn't actually measure wall-clock per pipeline. The S41 spec promised it does, but the implementation defers (the script prints `TBD`). This is a S41 follow-up, not S44.
- **Other demo SQL rewrites** (the stash has `examples/performance/fibonacci.sql` minor changes that aren't strictly necessary).

## File structure

| File | Action |
|---|---|
| `examples/performance/prime_sieve.sql` | Replace with stash's 20-phase version |
| `examples/performance/multi_stream_analytics.sql` | Replace with stash's simplified version |
| `scripts/demo-perf.sh` | Modify (only if stash's version is correct; otherwise skip) |

No new files. No test changes (the demo SQLs are not unit-tested; their correctness is verified by `bee run` integration).

## Acceptance criteria

- [x] `cargo build --workspace` green
- [x] `cargo test --workspace` ≥ 425 passed, 0 failed
- [x] `bee run examples/performance/prime_sieve.sql` runs end-to-end in **< 1s** and prints `n_primes = 1229` (π(10⁴))
- [x] `bee run examples/performance/multi_stream_analytics.sql` runs end-to-end and emits 10 rows of aggregation

## What actually landed (vs. spec)

| File | Stash WIP | What S44 applied | Why |
|---|---|---|---|
| `examples/performance/prime_sieve.sql` | 73 lines, 20 phases covering primes ≤ 71, range 10⁸ | **69 lines, 25 phases covering primes ≤ 100, range 10⁴** | The stash's 20 phases sieve through primes ≤ 71, but √10⁶ ≈ 1000, so the sieve was incomplete (e.g., 73² = 5329 wasn't filtered → wrong `n_primes`). 25 phases covering primes ≤ 100 makes the sieve correct for N = 10⁴ (√10⁴ = 100). Also reduced the range from 10⁸ to 10⁴ so the demo finishes in ~0.5s instead of 3 minutes. |
| `examples/performance/multi_stream_analytics.sql` | 26 lines using `LEFT ASOF JOIN ... WINDOW TUMBLING` | **Reverted to HEAD (66 lines using INNER JOIN + GROUP BY)** | The stash's version uses `window_start(c.ts, INTERVAL '1' MINUTE)` and `LEFT ASOF JOIN`, which DataFusion 50 cannot parse. The HEAD version uses plain `INNER JOIN` + `GROUP BY`, which runs end-to-end. |
| `examples/performance/fibonacci.sql` | (small comment cleanup) | **Reverted to HEAD** | Per spec, low-value change; the fibonacci demo's known pre-existing `handler returned -1` issue is unrelated to S44. |
| `scripts/demo-perf.sh` | Rewritten for multi-node (`BEE_NODES`, `scripts/load-plugin.sh`, `bee deploy`, `bee jobs wait`) | **Skipped** | The stash version references `scripts/load-plugin.sh` (does not exist), `bee deploy` (does not exist), `bee jobs wait` (does not exist). The HEAD version uses `bee run` correctly. |

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `prime_sieve.sql` runs in < 1s on a single node | ✓ (S44) | N — depends on hardware |
| `multi_stream_analytics.sql` runs end-to-end | ✓ (S44) | N |
| Full 10⁸ sieve behind `BEE_FULL_SIEVE=1` | — | N — S44.x follow-up |
| `scripts/demo-perf.sh` rewritten for multi-node cluster | — | N — S33.1 follow-up (depends on `bee deploy` / `bee jobs wait` being implemented) |

## Related work

- **S41** (performance showcase) — the parent; S44 is its "demo runs in 5 minutes" follow-up.
- **S45** (`.gitignore` cleanup) — independent.
- **S43** (Plugin KV port) — independent (already done).

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| Trim to primes ≤ 71 (20 phases) or to a different limit? | **Primes ≤ 71** — matches the stash; runs in < 5s | A larger limit (e.g., ≤ 200) would take longer and not measurably exercise more code paths |
| Apply the `fibonacci.sql` comment cleanup? | **Skip** — low value, stale comment is a docs nit | Keep S44 focused on the trim + analytics simplify |
| Apply the `scripts/demo-perf.sh` update? | **Apply if stash version is sensible; skip if it's noise** | Will inspect when applying |

If any of these decisions should change, the user can override during the spec review.