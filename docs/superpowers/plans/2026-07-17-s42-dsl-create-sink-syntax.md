# S42 — DSL `CREATE SINK` Syntax Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `CREATE SINK <name> AS <body>` SQL statement that desugars to `<body>` + `EMIT INTO <name>`, with one-SINK-per-SQL MVP constraint and strict-mode `use <name>;` enforcement.

**Architecture:** Apply the stash's WIP (adds `EmitTarget::Plugin(String)` + `CreateKind::Sink` + `strip_create_sink` skeleton) as Task 1's starting point. Refine `strip_create_sink` to append `EMIT INTO <name>` after the body. Add a strict-mode check in `check_strict_mode` that mirrors the existing source-side `use <name>;` validation. Tests are TDD-style: write the failing test, implement the minimal code, verify, commit.

**Tech Stack:** Rust, `regex` (for `strip_emit_into`'s case-insensitive matching — already in use), `serde` for the new enum variant, `datafusion` (unchanged — the rewrite must remain DataFusion-parseable).

---

## File Structure

| File | Action | Purpose |
|---|---|---|
| `crates/bee-dsl-sql/src/preprocess.rs` | Modify | Add `CreateKind::Sink`, refine `strip_create_sink`, extend `find_create_statement`, add strict-mode arm |
| `crates/bee-dsl-sql/src/preprocess.rs::tests` | Modify | 5 new tests + 1 refresh |
| `crates/bee-dsl-sql/src/lib.rs` | Modify (small) | Re-export `strip_create_sink` (already done in stash; verify) |
| `crates/bee-dsl-sql/src/physical.rs` | No change | Stash's `EmitTarget::Plugin(name)` arm already prints placeholder |

The plan has 5 Tasks. Task 1 is the stash-apply (one mechanical edit). Tasks 2-4 are TDD (failing test → minimal impl → verify → commit). Task 5 is the manual sanity + final test pass + commit.

---

## Task 1: Apply stash's DSL WIP (the starting point)

**Files:**
- Modify: `crates/bee-dsl-sql/src/preprocess.rs` (apply stash's diff)
- Modify: `crates/bee-dsl-sql/src/lib.rs` (apply stash's diff)
- Modify: `crates/bee-dsl-sql/src/physical.rs` (apply stash's diff)

The stash's WIP adds three pieces:

1. `EmitTarget::Plugin(String)` enum variant in `preprocess.rs`
2. `strip_create_sink` skeleton + `CreateKind::Sink` + `find_create_statement` extension
3. `preprocess_sql_v2` calls `strip_create_sink`
4. `physical::run_pipeline_with_config` matches `EmitTarget::Plugin(name)` (placeholder print)

- [ ] **Step 1.1: Apply the stash's DSL files via `git checkout stash@{0} --`**

Run:

```bash
git checkout stash@{0} -- crates/bee-dsl-sql/
git status
```

Expected: 3 files modified (preprocess.rs, lib.rs, physical.rs). No other files touched.

- [ ] **Step 1.2: Build to verify the WIP compiles**

Run: `cargo build -p bee-dsl-sql 2>&1 | tail -5`. Expected: clean build (the stash's WIP may have leftover `eprintln!` debug calls but should compile).

If there are compile errors due to stale state (e.g., the stash references things that don't exist on HEAD), the next step fixes them.

- [ ] **Step 1.3: Remove `eprintln!` debug calls from the stash's WIP**

The stash's `lib.rs::preprocess_sql_v2` has `eprintln!("=== DEBUG: ...")` calls (visible in the stash diff). These are debug noise; remove them.

In `crates/bee-dsl-sql/src/lib.rs`, inside `preprocess_sql_v2`, remove the two `eprintln!("=== DEBUG: ...")` lines (and the empty `let (_sink_name, after_sink) = strip_create_sink(&after_create);` line if it's unused — see Step 1.4).

Run: `cargo build -p bee-dsl-sql 2>&1 | tail -3`. Expected: clean build, no eprintln calls.

- [ ] **Step 1.4: Run the existing test suite to confirm no regression**

Run: `cargo test -p bee-dsl-sql 2>&1 | grep -E "^test result|FAILED" | head -10`. Expected: all existing tests still pass (the stash's WIP didn't add tests; we're just confirming baseline integrity).

- [ ] **Step 1.5: Commit the stash-apply**

```bash
git add crates/bee-dsl-sql/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S42 Task 1: apply stash's CREATE SINK WIP (EmitTarget::Plugin + CreateKind::Sink + strip_create_sink skeleton)"
```

---

## Task 2: `strip_create_sink` appends `EMIT INTO <name>` (TDD)

**Files:**
- Modify: `crates/bee-dsl-sql/src/preprocess.rs` (refine `strip_create_sink` + add 1 new test)
- Modify: `crates/bee-dsl-sql/src/preprocess.rs::tests` (add 1 new test + refresh 1 existing)

Currently `strip_create_sink` returns `(name, body_substituted_sql)` — i.e., it strips the `CREATE SINK foo AS <body>` and replaces with `<body>`. The Plugin arm in `physical::run_pipeline_with_config` never fires because there's no `EMIT INTO` left to strip.

We refine `strip_create_sink` to return `(name, body_with_emit_into_appended_sql)` — i.e., append `\nEMIT INTO <name>\n` after the body.

- [ ] **Step 2.1: Write the failing test (RED)**

In `crates/bee-dsl-sql/src/preprocess.rs::tests`, add a new test (place it near the other `strip_create_*` tests):

```rust
#[test]
fn strip_create_sink_appends_emit_into_target() {
    let sql = "CREATE SINK foo AS SELECT * FROM bar;";
    let (name, rewritten) = strip_create_sink(sql);
    assert_eq!(name, Some("foo".to_string()));
    // The body must still be present.
    assert!(
        rewritten.contains("SELECT * FROM bar"),
        "rewritten must contain the body; got: {rewritten}"
    );
    // The SINK must have been appended as `EMIT INTO foo`.
    assert!(
        rewritten.contains("EMIT INTO foo"),
        "rewritten must contain `EMIT INTO foo`; got: {rewritten}"
    );
    // The `CREATE SINK foo AS` prefix must have been stripped.
    assert!(
        !rewritten.contains("CREATE SINK"),
        "rewritten must NOT contain `CREATE SINK`; got: {rewritten}"
    );
}
```

- [ ] **Step 2.2: Run the test to verify it fails (RED)**

Run: `cargo test -p bee-dsl-sql --lib strip_create_sink_appends_emit_into_target 2>&1 | tail -5`. Expected: FAIL — the current `strip_create_sink` substitutes the body but does NOT append `EMIT INTO foo`.

- [ ] **Step 2.3: Refine `strip_create_sink` to append `EMIT INTO <name>`**

In `crates/bee-dsl-sql/src/preprocess.rs`, replace the function body of `strip_create_sink`:

```rust
/// Strip `CREATE SINK <name> AS <body>` and rewrite to
/// `<body>` followed by `EMIT INTO <name>`. Returns
/// `(Some(name), rewritten_sql)`. If no `CREATE SINK` is
/// found, returns `(None, original_sql)`.
///
/// MVP constraint: exactly one `CREATE SINK` per SQL. If
/// more than one is found, returns `(None, original_sql)`
/// so the downstream DataFusion parser will surface a
/// clean error.
pub fn strip_create_sink(sql: &str) -> (Option<String>, String) {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    let mut hit_name: Option<String> = None;

    while let Some(hit) = find_create_statement(rest) {
        if hit.kind == CreateKind::Sink {
            // MVP: only one SINK allowed.
            if hit_name.is_some() {
                return (None, sql.to_string());
            }
            // Output everything before the CREATE statement,
            // then the body (so DataFusion compiles the body),
            // then `EMIT INTO <name>` so the existing
            // strip_emit_into arm picks it up.
            out.push_str(&rest[..hit.start]);
            out.push_str(&hit.body);
            out.push_str("\nEMIT INTO ");
            out.push_str(&hit.name);
            out.push('\n');
            hit_name = Some(hit.name);
            rest = &rest[hit.end..];
        } else {
            // Non-SINK CREATE statement: keep verbatim.
            out.push_str(&rest[..hit.end]);
            rest = &rest[hit.end..];
        }
    }
    out.push_str(rest);
    (hit_name, out)
}
```

- [ ] **Step 2.4: Run the test (GREEN)**

Run: `cargo test -p bee-dsl-sql --lib strip_create_sink_appends_emit_into_target 2>&1 | tail -5`. Expected: PASS.

- [ ] **Step 2.5: Refresh the existing `strip_create_sink_extracts_name_and_body` test (if it exists)**

The stash might have a test that asserts the OLD (body-only) behavior. If so, refresh it to assert the NEW behavior (body + EMIT INTO appended). Search for any test referencing `strip_create_sink`:

```bash
grep -n "strip_create_sink" crates/bee-dsl-sql/src/preprocess.rs
```

If you find an existing test that asserts `rewritten == "<body>"` (without the EMIT INTO), update its assertion to:

```rust
assert!(rewritten.contains("EMIT INTO"));
```

- [ ] **Step 2.6: Commit**

```bash
git add crates/bee-dsl-sql/src/preprocess.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S42 Task 2: strip_create_sink appends EMIT INTO <name>"
```

---

## Task 3: Strict-mode check on SINK target (TDD)

**Files:**
- Modify: `crates/bee-dsl-sql/src/preprocess.rs` (extend `check_strict_mode`)
- Modify: `crates/bee-dsl-sql/src/preprocess.rs::tests` (add 2 tests)

The existing `check_strict_mode` rejects `binance.subscribe(...)` without a prior `use binance;`. We extend it to also reject `CREATE SINK <name> AS ...` without a prior `use <name>;`.

The function signature must change from `pub fn check_strict_mode(sql: &str) -> Result<(), String>` to take the list of `use` directives (or just take the SQL and re-parse). Simpler: take the SQL and re-parse the `use` directives internally.

- [ ] **Step 3.1: Read the existing `check_strict_mode` to understand its shape**

Run: `grep -n "fn check_strict_mode" crates/bee-dsl-sql/src/preprocess.rs`. Find the function definition (around line 100-200). Note its input/output shape.

- [ ] **Step 3.2: Write the failing tests (RED)**

In `crates/bee-dsl-sql/src/preprocess.rs::tests`, add 2 new tests:

```rust
#[test]
fn check_strict_mode_rejects_create_sink_without_use() {
    // No `use binance;` precedes the SINK → reject.
    let sql = "CREATE SINK binance AS SELECT 1;";
    let result = check_strict_mode(sql);
    assert!(result.is_err(), "expected strict-mode error for SINK without `use`");
    let err = result.unwrap_err();
    assert!(
        err.contains("binance") && err.to_lowercase().contains("use"),
        "expected error to mention `binance` and `use`; got: {err}"
    );
}

#[test]
fn check_strict_mode_accepts_create_sink_with_use() {
    // `use binance;` precedes the SINK → accept.
    let sql = "use binance;\nCREATE SINK binance AS SELECT 1;";
    let result = check_strict_mode(sql);
    assert!(result.is_ok(), "expected strict-mode OK with `use`; got: {result:?}");
}
```

- [ ] **Step 3.3: Run the tests to verify they fail (RED)**

Run: `cargo test -p bee-dsl-sql --lib check_strict_mode_rejects_create_sink check_strict_mode_accepts_create_sink 2>&1 | tail -10`. Expected: at least the "rejects" test fails — `check_strict_mode` doesn't currently check for SINK targets.

- [ ] **Step 3.4: Extend `check_strict_mode` to enforce SINK strict mode**

In `crates/bee-dsl-sql/src/preprocess.rs`, find the end of `check_strict_mode` and add a new arm that walks the SQL for `CREATE SINK <name>` and checks each `<name>` against the `use` directives already extracted.

Concretely, add at the end of `check_strict_mode` (before the `Ok(())`):

```rust
    // S42 strict-mode: every CREATE SINK target must have a
    // matching `use <name>;` directive. Mirror the source-side
    // check above.
    for line in sql.lines() {
        let trimmed = line.trim_start();
        if trimmed.to_ascii_uppercase().starts_with("CREATE SINK") {
            // Extract the target name (the first ASCII word
            // after `CREATE SINK`).
            let after_kw = trimmed["CREATE SINK".len()..].trim_start();
            let name_end = after_kw
                .find(|c: char| c.is_whitespace() || c == ';')
                .unwrap_or(after_kw.len());
            let sink_name = &after_kw[..name_end];
            if sink_name.is_empty() {
                return Err(
                    "CREATE SINK requires a target name (e.g. `CREATE SINK foo AS ...`)".into()
                );
            }
            if !uses.iter().any(|u| u.name == sink_name) {
                return Err(format!(
                    "CREATE SINK `{sink_name}` without prior `use {sink_name};`; \
                     strict mode requires every sink target to be declared via `use`"
                ));
            }
        }
    }
```

(Adjust the loop control / `uses` reference to match the existing code's shape — `check_strict_mode` already extracts `uses` somewhere; reuse that binding. If the existing function uses a different iteration pattern, adapt the loop.)

- [ ] **Step 3.5: Run the tests (GREEN)**

Run: `cargo test -p bee-dsl-sql --lib check_strict_mode_rejects_create_sink check_strict_mode_accepts_create_sink 2>&1 | tail -5`. Expected: both tests pass.

- [ ] **Step 3.6: Run the full `check_strict_mode` test set to confirm no regression**

Run: `cargo test -p bee-dsl-sql --lib check_strict_mode 2>&1 | grep -E "^test result|FAILED" | head -10`. Expected: all `check_strict_mode` tests pass (existing source-side tests + 2 new).

- [ ] **Step 3.7: Commit**

```bash
git add crates/bee-dsl-sql/src/preprocess.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S42 Task 3: check_strict_mode enforces CREATE SINK use <name>"
```

---

## Task 4: Multi-SINK error path (TDD)

**Files:**
- Modify: `crates/bee-dsl-sql/src/preprocess.rs::tests` (add 1 test)

Task 2 already handles multi-SINK in `strip_create_sink` (returns `(None, original_sql)` on the second SINK). Task 4 adds a test that locks down this behavior.

- [ ] **Step 4.1: Write the failing test (RED)**

In `crates/bee-dsl-sql/src/preprocess.rs::tests`, add:

```rust
#[test]
fn strip_create_sink_rejects_multiple_sinks() {
    let sql = "CREATE SINK foo AS SELECT 1;\nCREATE SINK bar AS SELECT 2;";
    let (name, rewritten) = strip_create_sink(sql);
    // MVP: only one SINK allowed; the second SINK causes
    // `strip_create_sink` to abort and return the original
    // SQL. DataFusion will then surface a clean parse error.
    assert_eq!(name, None);
    assert_eq!(rewritten, sql);
}
```

- [ ] **Step 4.2: Run the test (GREEN — should already pass from Task 2)**

Run: `cargo test -p bee-dsl-sql --lib strip_create_sink_rejects_multiple_sinks 2>&1 | tail -5`. Expected: PASS (Task 2's `strip_create_sink` returns `(None, original_sql)` on the second SINK).

If it fails, add the `if hit_name.is_some() { return (None, sql.to_string()); }` check at the top of the `if hit.kind == CreateKind::Sink` arm in `strip_create_sink` (already in Task 2's Step 2.3 code; verify it's there).

- [ ] **Step 4.3: Commit**

```bash
git add crates/bee-dsl-sql/src/preprocess.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S42 Task 4: lock down multi-SINK error path"
```

---

## Task 5: Final verification + workspace test pass + push

- [ ] **Step 5.1: Full workspace build**

Run: `cargo build --workspace 2>&1 | tail -5`. Expected: clean build.

- [ ] **Step 5.2: Full workspace test suite**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed:", p, "failed:", f, "ignored:", i}'`. Expected: `passed: 415+ failed: 0 ignored: 5` (the `+` comes from the 5 new S42 tests + the existing 415).

- [ ] **Step 5.3: Run `bee run` against a sample `CREATE SINK` SQL (manual sanity)**

Create a temporary file `/tmp/s42_demo.sql`:

```sql
use mock_input;
CREATE SINK mock_input AS
SELECT timestamp, sequence FROM generate_series(1, 3) AS t(n);
```

Run: `cargo run -p bee -- run /tmp/s42_demo.sql 2>&1 | tail -10`. Expected: the pipeline runs; the output includes `(emitted 3 row(s) to sink mock_input)` (placeholder from `physical::run_pipeline_with_config`'s Plugin arm).

If `generate_series` isn't available in the current bee-dsl-sql, swap to `SELECT 1 AS n` repeated 3 times. Adjust the SQL until the pipeline runs.

- [ ] **Step 5.4: Verify strict-mode error path with a manual run**

Create `/tmp/s42_bad.sql`:

```sql
CREATE SINK unknown_plugin AS SELECT 1;
```

Run: `cargo run -p bee -- run /tmp/s42_bad.sql 2>&1 | tail -5`. Expected: a clear error mentioning `unknown_plugin` and `use`.

- [ ] **Step 5.5: Update `CONTEXT.md` (optional)**

If `CONTEXT.md` has a Pipeline section that mentions `EMIT INTO`, add a 1-line note about `CREATE SINK` as the declarative alternative. Skip if the existing text is already general enough.

- [ ] **Step 5.6: Update `docs/stories.md` S42 acceptance criteria**

Flip all `[ ]` to `[x]` in the S42 section. Commit:

```bash
git add docs/stories.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S42: flip acceptance criteria to [x]"
```

- [ ] **Step 5.7: Push to remote**

```bash
git push origin main
```

- [ ] **Step 5.8: Optional — pop the stash and drop the now-applied WIP**

The stash's DSL changes are now committed. Drop the DSL portion from the stash:

```bash
git stash show stash@{0} -- crates/bee-dsl-sql/ > /dev/null 2>&1 \
  && git checkout HEAD -- crates/bee-dsl-sql/ \
  && git stash drop stash@{0}
```

(Caveat: if the stash contains other unrelated workstreams — DSL sink / kv.rs / prime_sieve / binance / docs/book — DO NOT drop the stash yet. Other stories S43-S45 still need it. Keep `stash@{0}` intact; only the DSL portion is now committed.)

---

## Self-Review

**1. Spec coverage:** Walked the S42 spec's in-scope items:
- Desugar CREATE SINK → body + EMIT INTO: Task 2 ✓
- One SINK per SQL: Task 4 ✓
- Strict-mode `use <name>;` check: Task 3 ✓
- Unit tests: Tasks 2, 3, 4 ✓
- CLI sanity: Task 5.3 ✓
- No demo SQL change: implicit (no task touches examples/) ✓
- Stash diff applied: Task 1 ✓
- Out-of-scope deferred: noted in spec ✓

**2. Placeholder scan:** Searched for "TBD" / "TODO" / "implement later" — none in the plan body. The "out of scope" deferrals in the spec are intentional.

**3. Type consistency:**
- `strip_create_sink(sql: &str) -> (Option<String>, String)` — consistent across Tasks 2 and 4
- `check_strict_mode(sql: &str) -> Result<(), String>` — unchanged signature; Task 3 extends the body
- `EmitTarget::Plugin(String)` — same variant the stash introduces; Task 5.3's manual sanity confirms the runtime path

**4. Ambiguity check:** Each test specifies concrete input + concrete expected output. The `strip_create_sink` body in Task 2 is complete Rust code. The strict-mode arm in Task 3 has a clear shape; the engineer adapts to the existing function structure.

---

## Estimated Total

- 5 Tasks
- ~7 commits (Tasks 1-4 each one, Task 5 verification + criteria flip + push = 3 commits)
- ~150-200 LOC net change in `crates/bee-dsl-sql/src/preprocess.rs` (the stash WIP is ~140 LOC; Task 2's append adds ~5 LOC; Task 3's strict-mode adds ~25 LOC; 5 new tests = ~80 LOC)
- Estimated wall-clock: 1-2 hours of focused TDD work
