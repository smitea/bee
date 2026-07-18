# S44 — S41 Demo Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Trim `prime_sieve.sql` to 20 phases (primes ≤ 71) so the S41 demo runs in < 5s. Simplify `multi_stream_analytics.sql` per the stash. No code logic changes — only demo SQL content + optional demo script update.

**Architecture:** Apply the stash's trimmed `prime_sieve.sql` and simplified `multi_stream_analytics.sql` via `git checkout stash@{0}`. Inspect the stash's `scripts/demo-perf.sh` and decide whether to apply it. Skip `fibonacci.sql` (low-value comment cleanup). Verify with `bee run` for both demos.

**Tech Stack:** SQL (DataFusion 50 dialect, Bee extensions), bash (demo script).

---

## File Structure

| File | Action | Purpose |
|---|---|---|
| `examples/performance/prime_sieve.sql` | Replace | 1229 phases → 20 phases (primes 2..71) |
| `examples/performance/multi_stream_analytics.sql` | Replace | Simplified comments + bumped event counts + renamed `joined` → `per_minute` |
| `examples/performance/fibonacci.sql` | No change | Skip per spec |
| `scripts/demo-perf.sh` | Inspect + conditionally replace | Apply if stash version is sensible; skip if it's noise |

3 Tasks. Task 1 applies the two SQL files and verifies each runs end-to-end. Task 2 inspects and conditionally applies the demo script. Task 3 runs the full workspace test + pushes.

---

## Task 1: Apply trimmed `prime_sieve.sql` + simplified `multi_stream_analytics.sql`

**Files:**
- Modify: `examples/performance/prime_sieve.sql` (apply from stash)
- Modify: `examples/performance/multi_stream_analytics.sql` (apply from stash)

- [ ] **Step 1.1: Apply the two SQL files via `git checkout stash@{0} --`**

Run:

```bash
git checkout stash@{0} -- examples/performance/prime_sieve.sql examples/performance/multi_stream_analytics.sql
git status
```

Expected: 2 files modified. `examples/performance/fibonacci.sql` should NOT be in the diff.

- [ ] **Step 1.2: Verify the new `prime_sieve.sql` content**

Run: `wc -l examples/performance/prime_sieve.sql`. Expected: ~70 lines (was 3707). The new file should have exactly 20 `CREATE VIEW sieved_X AS` statements for primes 2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71.

Verify: `grep -c "CREATE VIEW sieved_" examples/performance/prime_sieve.sql`. Expected: 21 (20 phases + the final `prime_count` view).

- [ ] **Step 1.3: Verify the new `multi_stream_analytics.sql` content**

Run: `head -10 examples/performance/multi_stream_analytics.sql`. Expected: should start with `-- 3 source streams of test events` (the stash's stripped comment), followed by `CREATE SOURCE clicks AS ...`.

- [ ] **Step 1.4: Run `prime_sieve.sql` end-to-end (verify < 5s + correct output)**

Run: `time cargo run -p bee -- run examples/performance/prime_sieve.sql 2>&1 | tail -5`. Expected: the run completes in < 5s and outputs `(emitted 1 row(s) to sink console)` with the table containing `n_primes = 20` (the number of primes ≤ 71).

(The output format may vary — it could print a table with `n_primes\n-\n20\n` or similar. The key assertion is `n_primes = 20` and the wall-clock.)

If the output shows `n_primes = 20`, the trim is correct. If it shows any other number, the stash's `prime_count` view is wrong — investigate.

- [ ] **Step 1.5: Run `multi_stream_analytics.sql` end-to-end**

Run: `cargo run -p bee -- run examples/performance/multi_stream_analytics.sql 2>&1 | tail -10`. Expected: the run completes (the per-minute aggregation emits some rows; the exact count depends on the synthetic event stream).

If the run fails, the stash's SQL may have a syntax error — investigate.

- [ ] **Step 1.6: Confirm no DSL preprocessor regression**

Run: `cargo test -p bee-dsl-sql --lib 2>&1 | grep -E "^test result|FAILED" | head -3`. Expected: 87+ tests pass (matches the current baseline after S42).

- [ ] **Step 1.7: Commit**

```bash
git add examples/performance/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S44 Task 1: trim prime_sieve.sql to 20 phases + simplify multi_stream_analytics.sql"
```

---

## Task 2: Inspect + conditionally apply `scripts/demo-perf.sh`

**Files:**
- Modify (conditional): `scripts/demo-perf.sh`

- [ ] **Step 2.1: Inspect the stash's `scripts/demo-perf.sh`**

Run: `git show stash@{0}:scripts/demo-perf.sh 2>&1 | head -100`. Compare against HEAD's version (`cat scripts/demo-perf.sh | head -100`).

Look for:
- Is the stash version sensible (real perf measurement, reasonable table formatting)?
- Does it reference demos that no longer exist or have changed paths?
- Does it assume hardcoded timing numbers that would lie after the prime_sieve trim?

Decision criteria:
- **Apply** if the stash version is clearly an improvement (real timing, correct demos, no hardcoded lies).
- **Skip** if the stash version is half-done, has stale references, or doesn't materially improve the script.

- [ ] **Step 2.2: Apply or skip**

If applying:

```bash
git checkout stash@{0} -- scripts/demo-perf.sh
```

If skipping: leave the script untouched and document in the commit message that the stash version was rejected.

- [ ] **Step 2.3: Verify the script runs end-to-end (if applied)**

If applied, run the script:

```bash
time bash scripts/demo-perf.sh 2>&1 | tail -30
```

Expected: the script runs all 3 demos in sequence and prints a summary table at the end. The trimmed prime_sieve should complete in seconds.

If the script fails, apply minimal fixes (e.g., comment out a stale demo reference) until it runs.

- [ ] **Step 2.4: Commit (if applied)**

```bash
git add scripts/demo-perf.sh
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S44 Task 2: update scripts/demo-perf.sh for trimmed demos"
```

(If skipped, no commit.)

---

## Task 3: Final verification + push

- [ ] **Step 3.1: Full workspace build**

Run: `cargo build --workspace 2>&1 | tail -5`. Expected: clean build.

- [ ] **Step 3.2: Full workspace test suite**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed:", p, "failed:", f, "ignored:", i}'`. Expected: `passed: 425+ failed: 0 ignored: 5` (no new test changes in S44; baseline preserved).

- [ ] **Step 3.3: Run both demos once more for a final sanity check**

```bash
cargo run -p bee -- run examples/performance/prime_sieve.sql 2>&1 | tail -3
cargo run -p bee -- run examples/performance/multi_stream_analytics.sql 2>&1 | tail -3
cargo run -p bee -- run examples/performance/fibonacci.sql 2>&1 | tail -3
```

Expected: all 3 demos complete without error.

- [ ] **Step 3.4: Update S44 spec acceptance criteria**

Edit `docs/superpowers/specs/2026-07-17-s44-prime-sieve-trim-design.md` and flip all `[ ]` to `[x]`. Commit:

```bash
git add docs/superpowers/specs/2026-07-17-s44-prime-sieve-trim-design.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S44: flip acceptance criteria to [x]"
```

- [ ] **Step 3.5: Update `docs/stories.md` S44 acceptance criteria**

Edit `docs/stories.md` (S44 section, line ~1176). Flip the `[ ]` to `[x]`. Commit:

```bash
git add docs/stories.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "stories.md: S44 acceptance criteria flipped"
```

- [ ] **Step 3.6: Push to remote**

```bash
git push origin main
```

---

## Self-Review

**1. Spec coverage:** Walked the S44 spec's in-scope items:
- Trimmed `prime_sieve.sql`: Task 1 ✓
- Simplified `multi_stream_analytics.sql`: Task 1 ✓
- Optional `scripts/demo-perf.sh` update: Task 2 ✓
- Skip `fibonacci.sql`: explicit ✓
- Verification (build + test + manual): Tasks 1.6, 3.1, 3.2 ✓

**2. Placeholder scan:** Searched for "TBD" / "TODO" — none in the plan body.

**3. Type consistency:** No type changes in S44 (no Rust code touched).

**4. Ambiguity check:** The demo SQLs' expected outputs are concrete (`n_primes = 20`, non-empty per-minute aggregation). The demo script decision is explicit (apply or skip based on inspection).

---

## Estimated Total

- 3 Tasks
- 2-4 commits (Task 1 = 1, Task 2 = 0-1, Task 3 = 3 verification)
- ~3700 LOC removed from `prime_sieve.sql` (1229 phases → 20 phases)
- Estimated wall-clock: 15-30 minutes