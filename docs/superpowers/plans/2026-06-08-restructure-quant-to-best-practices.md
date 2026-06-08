# Restructure · Move S33–S40 to `best-practices/quant/` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure the repo so the main codebase is generic (S17, S33 plugin infra, S33-deferred FFI, future S41 demo) and the quant-trading reference implementation lives in `docs/best-practices/quant/` + `plugins/quant/`. Rename 5 plugins to drop the `-mock` suffix. Single commit.

**Architecture:** 5 sections — (1) plugin rename + move (5 plugins to `plugins/quant/`, drop `-mock`), (2) docs migration (ADR-0011 + S33 wrap-up + S33-deferred design/plan to `docs/best-practices/quant/`), (3) create `docs/best-practices/quant/{README.md, stories.md}`, (4) update main docs (stories.md, product-design.md, README.md, S17 design/plan, bee plugin list CLI), (5) consolidate as a single commit.

**Tech Stack:** Git, Cargo workspace, no new Rust deps.

**Reference docs:**
- Design: `docs/superpowers/specs/2026-06-08-restructure-quant-to-best-practices-design.md`
- Source-of-truth: `docs/stories.md` (lines 483–936 for S33–S40; lines 1461+ for S41)
- `docs/product-design.md` (Scenario A lines 103–122; Scenario D lines 144–)
- `README.md` (lines 1–113)

**Pre-flight (read these before starting):**
- `plugins/bee-plugin-binance-mock/Cargo.toml` + `src/lib.rs` (representative; the other 4 are similar)
- `docs/adr/0011-stream-identity-and-backfill.md`
- `docs/superpowers/specs/2026-06-08-s33-deferred-ffi-design.md`
- `bee/src/main.rs` lines 640–700 (the `bee plugin list` subcommand)
- `Cargo.toml` workspace `members` list

**Working-tree state:** clean, on `main` at `66c4253` (the design commit).

---

## File structure (target)

**Renamed (5):**
- `plugins/bee-plugin-binance-mock/` → `plugins/quant/bee-plugin-binance/`
- `plugins/bee-plugin-google-news-mock/` → `plugins/quant/bee-plugin-google-news/`
- `plugins/bee-plugin-influxdb-mock/` → `plugins/quant/bee-plugin-influxdb/`
- `plugins/bee-plugin-mongodb-mock/` → `plugins/quant/bee-plugin-mongodb/`
- `plugins/bee-plugin-ta-lib-mock/` → `plugins/quant/bee-plugin-ta-lib/`

**Moved (6):**
- `docs/adr/0011-stream-identity-and-backfill.md` → `docs/best-practices/quant/adr/`
- `docs/superpowers/specs/2026-06-08-s33-deferred-ffi-design.md` → `docs/best-practices/quant/specs/`
- `docs/superpowers/plans/2026-06-08-s33-deferred-ffi.md` → `docs/best-practices/quant/plans/`
- `examples/quant_btc_macd.sql` → `docs/best-practices/quant/examples/`
- `examples/quant_btc_sentiment.sql` → `docs/best-practices/quant/examples/`
- `scripts/demo-quant-prod.sh` → `docs/best-practices/quant/scripts/`

**Created (2):**
- `docs/best-practices/quant/README.md`
- `docs/best-practices/quant/stories.md` (extracted from main)

**Updated in place (8):**
- `Cargo.toml` (workspace `members`)
- `docs/stories.md` (remove S33–S40, reposition S41)
- `docs/product-design.md` (remove Scenario A, promote Scenario D)
- `README.md` (point to S41, mention best-practices)
- `docs/superpowers/specs/2026-06-07-s17-stream-signature-design.md` (light edit; drop BTC in acceptance criteria; update ADR-0011 link)
- `docs/superpowers/plans/2026-06-07-s17-stream-signature.md` (same)
- `docs/adr/0003-producer-pipeline-pattern.md` + `0010-datasource-managed-entity.md` (update ADR-0011 link if cross-referenced)
- `bee/src/main.rs` (update `bee plugin list` subcommand's hardcoded plugin names)

**Boundary responsibilities:**
- Each renamed plugin's `Cargo.toml` `name` field is the source of truth for the crate name and the cdylib artifact name.
- The `bee plugin list` subcommand's hardcoded list must match the new plugin names exactly (else `bee demo-quant-prod.sh` will fail in CI).
- `docs/best-practices/quant/stories.md` is a verbatim copy of the S33–S40 sections from main `docs/stories.md`, with cross-references rewritten to point to the main repo.

---

## Task 1: Rename + move the 5 plugins (mechanical)

**Files:**
- 5 plugin directories (each has `Cargo.toml` + `src/lib.rs` + `tests/loader_smoke.rs`)

- [ ] **Step 1: `git mv` the 5 plugin directories**

```bash
cd /Users/shaw/Developer/rust/bee && \
  git mv plugins/bee-plugin-binance-mock      plugins/quant/bee-plugin-binance && \
  git mv plugins/bee-plugin-google-news-mock  plugins/quant/bee-plugin-google-news && \
  git mv plugins/bee-plugin-influxdb-mock     plugins/quant/bee-plugin-influxdb && \
  git mv plugins/bee-plugin-mongodb-mock      plugins/quant/bee-plugin-mongodb && \
  git mv plugins/bee-plugin-ta-lib-mock       plugins/quant/bee-plugin-ta-lib
mkdir -p plugins/quant  # no-op if the git mv above already created it
git status --short  # should show 10 renamed files
```

- [ ] **Step 2: Edit each plugin's `Cargo.toml`: drop `-mock` from `name` and update description**

For each of the 5 plugins, in `plugins/quant/bee-plugin-{name}/Cargo.toml`:

Change `name = "bee-plugin-{name}-mock"` to `name = "bee-plugin-{name}"`.

Change the `description` to drop "mock" / "sine-wave" / "test fixture" / "in-process" language. Use this template:

For `binance`:
```toml
description = "Bee 量化交易参考实现:实时价格流 InputAdapter。Plugin 结构是 production-grade (cdylib + FFI vtable);实际数据源是 sine-wave 占位,等 S34 production 实装时替换。"
```

For `google-news`:
```toml
description = "Bee 量化交易参考实现:实时新闻流 InputAdapter。Plugin 结构是 production-grade;实际数据源是合成新闻占位。"
```

For `influxdb`:
```toml
description = "Bee 量化交易参考实现:实时时序 OutputAdapter。Plugin 结构是 production-grade;实际 sink 是日志文件占位。"
```

For `mongodb`:
```toml
description = "Bee 量化交易参考实现:文档型 OutputAdapter。Plugin 结构是 production-grade;实际 sink 是 jsonl 文件占位。"
```

For `ta-lib`:
```toml
description = "Bee 量化交易参考实现:技术分析 Handler 集合 (MACD/EMA/decision_tree/sentiment_analyzer)。Plugin 结构是 production-grade;实际计算是简化版占位。"
```

- [ ] **Step 3: Verify each plugin's `Cargo.toml` no longer contains `-mock` or `mock`**

```bash
cd /Users/shaw/Developer/rust/bee && \
  grep -l "mock\|Mock" plugins/quant/*/Cargo.toml || echo "clean"
```

Expected: prints "clean".

- [ ] **Step 4: Verify workspace still builds (just the plugins, not the rest)**

```bash
cd /Users/shaw/Developer/rust/bee && cargo check -p bee-plugin-binance -p bee-plugin-google-news -p bee-plugin-influxdb -p bee-plugin-mongodb -p bee-plugin-ta-lib 2>&1 | tail -10
```

Expected: 5 errors, all `error: package ID specification `bee-plugin-{name}-mock` did not match any packages` (because `Cargo.toml` workspace `members` still points to the old paths; we'll fix that in Task 2). DO NOT COMMIT YET.

---

## Task 2: Update workspace `Cargo.toml` members + update plugin `src/lib.rs` test names

**Files:**
- `Cargo.toml` (workspace)
- 5 plugin `src/lib.rs` files (drop "mock" from test names)

- [ ] **Step 1: Update workspace `Cargo.toml` `members` list**

Open `/Users/shaw/Developer/rust/bee/Cargo.toml`. In the `members = [...]` list, replace the 5 plugin paths:

```toml
members = [
    "crates/bee-types",
    "crates/bee-transport",
    "crates/bee-codec",
    "crates/bee-session",
    "crates/bee-runtime",
    "crates/bee-control",
    "crates/bee-registry",
    "crates/bee-dsl-sql",
    "crates/bee-adapter",
    "crates/bee-plugin-sdk",
    "crates/bee-kv-test",
    "bee",
    "plugins/quant/bee-plugin-binance",
    "plugins/quant/bee-plugin-google-news",
    "plugins/quant/bee-plugin-influxdb",
    "plugins/quant/bee-plugin-mongodb",
    "plugins/quant/bee-plugin-ta-lib",
]
```

- [ ] **Step 2: Verify the workspace builds**

```bash
cd /Users/shaw/Developer/rust/bee && cargo check --workspace 2>&1 | tail -10
```

Expected: clean (or only the pre-existing warnings).

- [ ] **Step 3: For each plugin's `src/lib.rs`, drop "mock" / "Mock" from test names + module-level docs**

For each of the 5 plugins, in `plugins/quant/bee-plugin-{name}/src/lib.rs`:

(a) In the module-level `//!` doc, replace phrases like "mock plugin" / "S33 mock" / "sine-wave generator" with "reference implementation" / "production-grade scaffold" / "sine-wave placeholder (will be replaced by real Binance WS in S34)".

(b) In `#[cfg(test)] mod tests`, rename any test function containing "mock" or "Mock":
- `sine_wave_mock` → `sine_wave_placeholder`
- `vtable_next_returns_sine_wave_event` (binance) → `vtable_next_returns_tick_event`
- Any other test with "mock" in the name → drop the "mock" word

- [ ] **Step 4: Verify the 5 plugins' tests pass**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-binance -p bee-plugin-google-news -p bee-plugin-influxdb -p bee-plugin-mongodb -p bee-plugin-ta-lib 2>&1 | tail -10
```

Expected: all tests pass; same count as before (just renamed).

- [ ] **Step 5: Run the full workspace tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | grep -E "test result|error" | tail -10
```

Expected: 354 passing, 0 failing (same as before the restructure).

---

## Task 3: Update `bee plugin list` CLI subcommand (drop `-mock`)

**Files:**
- `bee/src/main.rs` (around line 640–700)

- [ ] **Step 1: Find the `bee plugin list` subcommand's hardcoded plugin list**

```bash
cd /Users/shaw/Developer/rust/bee && grep -n "bee-plugin.*-mock\|plugin-binance\|plugin-google\|plugin-influx\|plugin-mongo\|plugin-ta-lib" bee/src/main.rs
```

Identify the 5 hardcoded names + the `run_plugin_cli` function (or whatever the subcommand handler is named).

- [ ] **Step 2: Drop `-mock` from the 5 plugin names**

Replace:
- `bee-plugin-binance-mock` → `bee-plugin-binance`
- `bee-plugin-google-news-mock` → `bee-plugin-google-news`
- `bee-plugin-influxdb-mock` → `bee-plugin-influxdb`
- `bee-plugin-mongodb-mock` → `bee-plugin-mongodb`
- `bee-plugin-ta-lib-mock` → `bee-plugin-ta-lib`

(Adapt the variable name + cdylib artifact name in the same edit: `libbee_plugin_{name}_mock.dylib` → `libbee_plugin_{name}.dylib`.)

- [ ] **Step 3: Verify `bee plugin list` works**

```bash
cd /Users/shaw/Developer/rust/bee && cargo run -p bee --bin bee -- plugin list 2>&1 | head -20
```

Expected: prints the 5 plugins with their renamed names. The `built`/`missing` flag should reflect the actual file presence at `target/debug/libbee_plugin_{name}.dylib`.

- [ ] **Step 4: Verify the demo script (still in `scripts/` for now) also passes**

```bash
cd /Users/shaw/Developer/rust/bee && bash scripts/demo-quant-prod.sh 2>&1 | tail -15
```

Expected: 11/11 steps pass (with the renamed cdylib artifacts).

- [ ] **Step 5: Commit Tasks 1–3 as one commit (interim, before doc migration)**

```bash
cd /Users/shaw/Developer/rust/bee && git add plugins Cargo.toml bee && git -c user.name="opencode" -c user.email="opencode@local" commit -m "Restructure (interim): rename 5 plugins to drop -mock, move to plugins/quant/"
```

**Note**: this is the FIRST of 2 commits. The final consolidation (Task 7) will `git reset --soft 3d16622^` and squash everything into one commit. So this interim commit's history will be folded into the final commit.

---

## Task 4: Move ADR-0011 + S33 docs to `docs/best-practices/quant/`

**Files:**
- `docs/adr/0011-stream-identity-and-backfill.md` (move)
- `docs/superpowers/specs/2026-06-08-s33-deferred-ffi-design.md` (move)
- `docs/superpowers/plans/2026-06-08-s33-deferred-ffi.md` (move)
- `docs/adr/0003-producer-pipeline-pattern.md` + `0010-datasource-managed-entity.md` (update cross-refs)

- [ ] **Step 1: Move the 3 docs**

```bash
cd /Users/shaw/Developer/rust/bee && \
  mkdir -p docs/best-practices/quant/adr docs/best-practices/quant/specs docs/best-practices/quant/plans && \
  git mv docs/adr/0011-stream-identity-and-backfill.md docs/best-practices/quant/adr/ && \
  git mv docs/superpowers/specs/2026-06-08-s33-deferred-ffi-design.md docs/best-practices/quant/specs/ && \
  git mv docs/superpowers/plans/2026-06-08-s33-deferred-ffi.md docs/best-practices/quant/plans/
```

- [ ] **Step 2: Verify the 3 docs moved (no longer in original locations)**

```bash
cd /Users/shaw/Developer/rust/bee && \
  ls docs/adr/0011* 2>/dev/null || echo "ADR-0011 moved (good)"; \
  ls docs/superpowers/specs/2026-06-08-s33-deferred* 2>/dev/null || echo "S33-deferred spec moved (good)"; \
  ls docs/superpowers/plans/2026-06-08-s33-deferred* 2>/dev/null || echo "S33-deferred plan moved (good)"
```

Expected: 3 "moved (good)" messages.

- [ ] **Step 3: Update ADR-0011 cross-references in ADR-0003 + ADR-0010**

Open `docs/adr/0003-producer-pipeline-pattern.md` and `docs/adr/0010-datasource-managed-entity.md`. Search for any reference to `0011-stream-identity-and-backfill` or `adr/0011`.

If found, update the link to `../../best-practices/quant/adr/0011-stream-identity-and-backfill.md` (or the docs-relative equivalent).

If not found, skip.

- [ ] **Step 4: Verify the moved docs still render OK (no broken cross-refs)**

```bash
cd /Users/shaw/Developer/rust/bee && \
  grep -rn "0011-stream-identity\|adr/0011" docs/adr/ docs/product-design.md docs/stories.md 2>/dev/null
```

Expected: no broken references (either the references are updated, or there are no references).

---

## Task 5: Move examples + script to `docs/best-practices/quant/`

**Files:**
- `examples/quant_btc_macd.sql` (move)
- `examples/quant_btc_sentiment.sql` (move)
- `scripts/demo-quant-prod.sh` (move)

- [ ] **Step 1: Move the 3 files**

```bash
cd /Users/shaw/Developer/rust/bee && \
  mkdir -p docs/best-practices/quant/examples docs/best-practices/quant/scripts && \
  git mv examples/quant_btc_macd.sql docs/best-practices/quant/examples/ && \
  git mv examples/quant_btc_sentiment.sql docs/best-practices/quant/examples/ && \
  git mv scripts/demo-quant-prod.sh docs/best-practices/quant/scripts/
```

- [ ] **Step 2: Verify the moves + the demos are still executable**

```bash
cd /Users/shaw/Developer/rust/bee && \
  bash docs/best-practices/quant/scripts/demo-quant-prod.sh 2>&1 | tail -15
```

Expected: 11/11 steps pass. (The script doesn't reference its own path, so it runs from anywhere.)

- [ ] **Step 3: Verify `examples/` and `scripts/` are empty (or contain only S41 files if they exist)**

```bash
cd /Users/shaw/Developer/rust/bee && \
  ls examples/ scripts/ 2>/dev/null
```

Expected: empty (or "No such file or directory" if the directories were removed by `git mv` of their only contents).

- [ ] **Step 4: Commit Tasks 4–5 as one commit (interim)**

```bash
cd /Users/shaw/Developer/rust/bee && git add docs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "Restructure (interim): move ADR-0011 + S33-deferred docs + quant examples + demo script to docs/best-practices/quant/"
```

---

## Task 6: Create `docs/best-practices/quant/{README.md, stories.md}` + update main docs

**Files:**
- Create: `docs/best-practices/quant/README.md`
- Create: `docs/best-practices/quant/stories.md`
- Update: `docs/stories.md` (remove S33–S40, reposition S41, update dep graph + table + key milestones)
- Update: `docs/product-design.md` (remove Scenario A, promote Scenario D)
- Update: `README.md` (point to S41, mention best-practices)
- Update: `docs/superpowers/specs/2026-06-07-s17-stream-signature-design.md` (drop BTC in acceptance criteria; update ADR-0011 link)
- Update: `docs/superpowers/plans/2026-06-07-s17-stream-signature.md` (same)

- [ ] **Step 1: Create `docs/best-practices/quant/README.md`**

```markdown
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
- `examples/quant_btc_macd.sql` + `quant_btc_sentiment.sql` —
  two end-to-end SQL pipelines: BTC K-line + MACD/EMA (technical
  only) and BTC K-line + FinBERT sentiment + decision tree
  (technical + ML).
- `scripts/demo-quant-prod.sh` — architecture-level smoke demo
  for the FFI + runtime dispatching + 5 plugins + 2 pipelines.
- `specs/2026-06-07-s33-plugin-crates-design.md` +
  `specs/2026-06-08-s33-deferred-ffi-design.md` — the design
  specs for the 5 mock plugin scaffolds and the FFI wire format.
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
are placeholders. S34–S40 in `stories.md` replace them with real
implementations.

## See also

- Main repo `docs/stories.md` for the generic Bee story set
  (S0–S32, S41).
- Main repo `docs/adr/0001`–`0010` for the generic architecture
  decisions.
- Main repo `plugins/` for the S41 performance plugins (land in
  a future session).
```

- [ ] **Step 2: Create `docs/best-practices/quant/stories.md`**

The simplest path: read the S33 + S34–S40 sections from main `docs/stories.md`, copy them verbatim to this new file, and update cross-references.

```bash
cd /Users/shaw/Developer/rust/bee && \
  awk '/^### S33 · /,/^---$/' docs/stories.md > /tmp/s33.md && \
  awk '/^### S34 · /,/^---$/' docs/stories.md > /tmp/s34.md && \
  awk '/^### S35 · /,/^---$/' docs/stories.md > /tmp/s35.md && \
  awk '/^### S36 · /,/^---$/' docs/stories.md > /tmp/s36.md && \
  awk '/^### S37 · /,/^---$/' docs/stories.md > /tmp/s37.md && \
  awk '/^### S38 · /,/^---$/' docs/stories.md > /tmp/s38.md && \
  awk '/^### S39 · /,/^---$/' docs/stories.md > /tmp/s39.md && \
  awk '/^### S40 · /,/^---$/' docs/stories.md > /tmp/s40.md && \
  cat /tmp/s33.md /tmp/s34.md /tmp/s35.md /tmp/s36.md /tmp/s37.md /tmp/s38.md /tmp/s39.md /tmp/s40.md > docs/best-practices/quant/stories.md && \
  head -5 docs/best-practices/quant/stories.md
```

The output should start with `### S33 · ...`. If the awk extraction is brittle (e.g., some sections span multiple `---\n` lines), manually adjust the extraction pattern. The goal: the new file contains exactly the §S33 through §S40 sections, verbatim.

Add a header at the top of the new file:
```markdown
# Bee · Best Practices · Quant Trading · Stories

This is the quant-trading reference implementation story set
(S33 HITL milestone + S34–S40 production plugins + e2e deploy).
It complements the main repo's `docs/stories.md` (which covers
S0–S32, S41 — the generic Bee feature set + performance showcase).

Cross-references in these stories point back to the main repo's
`docs/stories.md` (e.g., "see S17" means "see main stories.md §S17").

---

```

- [ ] **Step 3: Update main `docs/stories.md`**

(a) In the dependency graph (around lines 54–62), remove all edges mentioning S33, S34, S35, S36, S37, S38, S39, S40.

(b) In the parallel-paths table (around line 81), remove the row for "H. Quant trading spike (prod)".

(c) In the body, REMOVE the §S33 through §S40 sections (they now live in `docs/best-practices/quant/stories.md`). Use `awk` to slice:
```bash
cd /Users/shaw/Developer/rust/bee && \
  awk '/^### S33 · / {exit} {print}' docs/stories.md > /tmp/stories_s0_s32.md && \
  awk '/^### S41 · /,0' /tmp/stories_s0_s32.md > /tmp/stories_s0_s32_plus_s41.md && \
  cat /tmp/stories_s0_s32.md | awk 'BEGIN{p=1} /^### S41 · /{p=0} p' > /tmp/stories_s0_s32_only.md && \
  cat /tmp/stories_s0_s32_only.md
```

(The exact awk pattern needs to be verified against the actual file structure. The MVP: remove the §S33 through §S40 sections from main `docs/stories.md`, leaving §S0–§S32 and §S41 in place.)

(d) In the "Key milestones" section, replace the S33 / S40 / S41 entries to point to `docs/best-practices/quant/stories.md` for S33–S40, and promote S41 to the primary demo slot:
```markdown
- **S41**: **Performance showcase (in flight)** — Fibonacci + prime sieve
  + multi-stream analytics demos run in < 5 min, with a measured
  performance table. This is the new primary demo of the main
  repo (replacing the quant demo that has moved to
  `docs/best-practices/quant/`).
```

(e) Add a footnote at the top of `docs/stories.md`:
```markdown
> **Note**: S33 (quant HITL milestone) + S34–S40 (production
> plugins + e2e deploy) are the quant-trading reference
> implementation; they live in
> [`docs/best-practices/quant/stories.md`](best-practices/quant/stories.md).
> The main repo's stories are now S0–S32 (generic Bee feature
> set) + S41 (performance showcase).
```

- [ ] **Step 4: Update `docs/product-design.md`**

(a) Remove the "### Scenario A: Quant decision pipeline" section (lines 103–122).

(b) Rename the existing "### Scenario D: Performance showcase" to "### Scenario A: Performance showcase (the 5-minute evaluator demo)" and move it to where Scenario A was.

(c) In the §"What's in the repo" section (around lines 50–70), update the description to mention `plugins/quant/` and `docs/best-practices/quant/` as the home for the quant reference implementation. The main repo's primary story is the S41 performance showcase.

(d) In the "Roadmap" / "User story" sections, replace the quant-specific language with generic S41 language.

- [ ] **Step 5: Update `README.md`**

(a) In the "Quickstart" section, replace the `scripts/demo-quant-prod.sh` reference with a forward-looking note:
```markdown
The canonical 5-minute end-to-end demo will land as
`scripts/demo-perf-prod.sh` (S41 performance showcase, in flight).
```

(b) In the docs table, update the `docs/stories.md` row to point to the new structure:
```markdown
| [docs/stories.md](docs/stories.md) | Generic Bee feature set: **33 implementation stories + S41 spike** (S0–S32 + performance showcase). Quant trading stories (S33–S40) live in [docs/best-practices/quant/stories.md](docs/best-practices/quant/stories.md). | Implementers |
```

(c) In the example-plugins section, replace the 5 quant plugins with a note:
```markdown
Example plugins under [plugins/quant/](plugins/quant/) are
quant-trading reference implementations (binance / google-news /
influxdb / mongodb / ta-lib). Their plugin STRUCTURE is
production-grade; the data sources are placeholders. They live
under [docs/best-practices/quant/](docs/best-practices/quant/) as
real-world business examples.
```

(d) Add a "Quant trading reference" section near the bottom:
```markdown
## Quant trading reference

The quant-trading implementation (S33 HITL milestone + S34–S40
production plugins + e2e deploy) is a large, real-world business
example. It lives in its own documentation section:

- [docs/best-practices/quant/](docs/best-practices/quant/) — stories,
  ADRs, examples, demo scripts, design specs.

The 5 reference plugin crates are at
[plugins/quant/](plugins/quant/) and are part of the workspace
members.
```

- [ ] **Step 6: Light edit `docs/superpowers/specs/2026-06-07-s17-stream-signature-design.md`**

(a) In §2 "Acceptance criteria" (line 499 area), replace the `binance.subscribe('BTC/USDT', '5min')` example with a generic reference:
```markdown
- [ ] Integration test: deploy Job A with `exchange.subscribe(symbol='X/Y', interval='5min')` — creates Producer
```
(use a generic exchange name; the underlying infrastructure is generic)

(b) In any reference to ADR-0011, update the link:
```markdown
[0011](./../../best-practices/quant/adr/0011-stream-identity-and-backfill.md)
```

(adjust the relative path; the spec lives in `docs/superpowers/specs/`, ADR-0011 lives in `docs/best-practices/quant/adr/`, so the relative path is `../../best-practices/quant/adr/0011-stream-identity-and-backfill.md`).

(c) In the "Acceptance" criteria for ADR-0011 reference, update the wording to point to the moved ADR.

- [ ] **Step 7: Light edit `docs/superpowers/plans/2026-06-07-s17-stream-signature.md` (same edits as Step 6)**

- [ ] **Step 8: Verify the docs are consistent**

```bash
cd /Users/shaw/Developer/rust/bee && \
  grep -c "binance\|google-news\|influxdb\|mongodb\|ta-lib\|quant" docs/stories.md docs/product-design.md README.md 2>/dev/null
```

Expected: small counts (a few mentions in the "see best-practices" footnotes; should NOT contain the full quant narrative).

- [ ] **Step 9: Verify the docs build OK + tests still pass**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build --workspace 2>&1 | tail -5 && cargo test --workspace 2>&1 | grep -E "test result" | tail -3
```

Expected: 354 passing tests, build clean.

- [ ] **Step 10: Commit Tasks 4–6 as one commit (interim)**

```bash
cd /Users/shaw/Developer/rust/bee && git add docs README.md && git -c user.name="opencode" -c user.email="opencode@local" commit -m "Restructure (interim): create best-practices/quant/README + stories; update main docs to drop quant narrative"
```

---

## Task 7: Final consolidation (single commit)

- [ ] **Step 1: Run the full workspace tests one more time**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | grep -E "test result" | tail -5
```

Expected: 354 passing, 0 failing.

- [ ] **Step 2: Look at the commit history since the design commit**

```bash
cd /Users/shaw/Developer/rust/bee && git log --oneline 66c4253..HEAD
```

Expected: ~4 commits (Tasks 1–3, Tasks 4–5, Task 6, etc.) + this design.

- [ ] **Step 3: Soft-reset + commit as a single "Restructure" commit**

```bash
cd /Users/shaw/Developer/rust/bee && git reset --soft 66c4253 && git -c user.name="opencode" -c user.email="opencode@local" commit -m "Restructure: move S33-S40 to best-practices/quant/, rename plugins, S41 becomes primary demo

Per docs/superpowers/specs/2026-06-08-restructure-quant-to-best-practices-design.md
(this session's design).

The repo's primary story is now the GENERIC Bee (S17 Producer
sharing + S33 plugin infra + S33-deferred FFI + S41 performance
showcase). The quant trading work is a real-world business
example that lives in its own documentation section.

## Section 1 — Plugins renamed and moved

5 mock plugins move to plugins/quant/ and drop the -mock suffix
(they're scaffolding for future production plugins, not throwaway
mocks):

- plugins/bee-plugin-binance-mock/      -> plugins/quant/bee-plugin-binance/
- plugins/bee-plugin-google-news-mock/  -> plugins/quant/bee-plugin-google-news/
- plugins/bee-plugin-influxdb-mock/     -> plugins/quant/bee-plugin-influxdb/
- plugins/bee-plugin-mongodb-mock/      -> plugins/quant/bee-plugin-mongodb/
- plugins/bee-plugin-ta-lib-mock/       -> plugins/quant/bee-plugin-ta-lib/

For each: Cargo.toml name + cdylib artifact rename, src/lib.rs
description updated to 'reference implementation' language, test
function names dropped 'mock' / 'sine-wave' references.

Workspace Cargo.toml members list updated. bee/src/main.rs
'bee plugin list' subcommand's hardcoded plugin list updated
to the 5 new names.

## Section 2 — Docs migration to docs/best-practices/quant/

Quant-flavored docs move out of the main docs/ tree:

- docs/adr/0011-stream-identity-and-backfill.md
  -> docs/best-practices/quant/adr/
- docs/superpowers/specs/2026-06-08-s33-deferred-ffi-design.md
  -> docs/best-practices/quant/specs/
- docs/superpowers/plans/2026-06-08-s33-deferred-ffi.md
  -> docs/best-practices/quant/plans/
- examples/quant_btc_macd.sql
  -> docs/best-practices/quant/examples/
- examples/quant_btc_sentiment.sql
  -> docs/best-practices/quant/examples/
- scripts/demo-quant-prod.sh
  -> docs/best-practices/quant/scripts/

ADR-0001..0010 stay in main docs/adr/ (generic decisions).
ADR-0011 is the quant-specific one (Binance WS backfill
semantics) and lives in best-practices/quant/.

## Section 3 — docs/best-practices/quant/ created

New directory with:
- README.md: pointer explaining the section's purpose
- stories.md: S33 (quant HITL milestone) + S34-S40 (production
  plugins + e2e deploy) extracted from main docs/stories.md
  (verbatim; cross-references rewritten to point back to main)

## Section 4 — Main docs updated

- docs/stories.md: S33-S40 sections removed (they live in
  best-practices/quant/stories.md now); S41 repositioned as
  the primary demo; dep graph + parallel-paths table + key
  milestones updated; footnote added pointing to
  best-practices/quant/.
- docs/product-design.md: 'Scenario A: Quant decision pipeline'
  removed; 'Scenario D: Performance showcase' renamed + moved
  to be the first scenario; 'What's in the repo' section updated.
- README.md: 'Quickstart' now points to the future S41 demo
  (scripts/demo-perf-prod.sh, in flight); example-plugins
  section points to plugins/quant/ + best-practices/quant/;
  new 'Quant trading reference' section near the bottom.
- docs/superpowers/specs/2026-06-07-s17-stream-signature-design.md
  + matching plan: light edit to replace the binance/BTC
  example in §2 acceptance criteria with a generic
  exchange.subscribe example; ADR-0011 cross-reference link
  updated to the new path.
- ADR-0003 + ADR-0010: cross-references to ADR-0011 updated to
  the new path (if present).

## Section 5 — Verification

- [x] cargo build --workspace clean
- [x] cargo test --workspace 354/354 passing
- [x] The 5 plugins only live under plugins/quant/ (not
  plugins/ directly)
- [x] docs/adr/ contains only ADR-0001..0010
- [x] docs/best-practices/quant/ has README, stories.md, adr/,
  examples/, scripts/, specs/, plans/
- [x] The 5 plugins' Cargo.toml name has no -mock suffix
- [x] docs/stories.md no longer contains the full quant narrative
- [x] docs/product-design.md has no 'Scenario A: Quant' section
- [x] The 'bee plugin list' subcommand lists the 5 renamed plugins
- [x] The demo script (now at
  docs/best-practices/quant/scripts/demo-quant-prod.sh) runs
  11/11 steps green

## What stays in main

S17 (StreamSignature / Producer/Subscriber / reconnect — generic)
+ S33 plugin infra (libloading + cdylib — generic) + S33-deferred
FFI (vtables, registry, wrappers — generic) + the core crates +
S41 (performance showcase — generic, in flight).

## Out of scope (deferred)

- S41 implementation (Fibonacci + prime sieve + multi-stream
  analytics). S41 stays as a future story in docs/stories.md;
  this session only restructures the project around it.
- The 5 quant plugins' bodies (Binance WS, NewsAPI, InfluxDB v2,
  MongoDB, yata/ta-lib). S34-S40 in best-practices/quant/stories.md
  will replace them in future sessions.
- 3-node cluster + e2e SQL deploy (S40 in
  best-practices/quant/stories.md).
- HITL seed-user walkthrough.

S17 commit (b680d3b), S33 wrap-up (22a9e39), and S33-deferred
(3d16622) commits all stay on main. The pre-existing warnings in
crates/bee-control/tests/{deploy_pipeline,raft_cluster}.rs are
out of scope; pre-date this commit."
```

- [ ] **Step 4: Verify the single commit**

```bash
cd /Users/shaw/Developer/rust/bee && git log --oneline 66c4253..HEAD
```

Expected: a single commit starting with `Restructure: move S33-S40 ...`.

- [ ] **Step 5: Final verification — structural greps**

```bash
cd /Users/shaw/Developer/rust/bee && \
  echo '=== 5 plugins only under plugins/quant/ ===' && \
  git ls-files | grep -E 'plugins/bee-plugin-(binance|google-news|influxdb|mongodb|ta-lib)' && \
  echo '=== ADR-0011 not in main docs/adr/ ===' && \
  (ls docs/adr/0011* 2>/dev/null && echo 'FAIL: ADR-0011 still in main') || echo 'OK: ADR-0011 moved' && \
  echo '=== best-practices/quant/ structure ===' && \
  ls docs/best-practices/quant/ && \
  echo '=== main docs/stories.md does not contain the full quant narrative ===' && \
  grep -c 'binance\|google-news\|influxdb\|mongodb\|ta-lib' docs/stories.md
```

Expected: 5 plugins only in `plugins/quant/`; ADR-0011 not in main; `best-practices/quant/` has all the expected subdirs; main `docs/stories.md` has a small count (just the "see best-practices" footnote).

- [ ] **Step 6: Re-run the demo script on the consolidated commit**

```bash
cd /Users/shaw/Developer/rust/bee && bash docs/best-practices/quant/scripts/demo-quant-prod.sh 2>&1 | tail -15
```

Expected: 11/11 steps pass.

- [ ] **Step 7: Final workspace test**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | grep -E "test result" | tail -3
```

Expected: 354 passing, 0 failing.

---

## Self-review checklist (run before claiming done)

- [ ] Spec coverage: Section 1 (Tasks 1–3), Section 2 (Tasks 4–5), Section 3 (Task 6 step 1–2), Section 4 (Task 6 step 3–7), Section 5 (Task 7).
- [ ] No placeholders: every step has actual content; the README.md + stories.md content is fully written.
- [ ] Type/file consistency: the 5 plugin names appear identically in `Cargo.toml`, `bee/src/main.rs`, and the demo script; the `docs/best-practices/quant/` structure matches what the README points to.
- [ ] DRY: OK.
- [ ] Commits: 1 final commit (after the soft-reset); the interim commits (Tasks 3, 5, 6) get folded in.
- [ ] YAGNI: no extra renames beyond the 5 plugins; no new docs beyond what's in the plan.
- [ ] No regressions: all 354 tests still pass; no new warnings.

## Out-of-scope items (do not address in this plan)

- S41 implementation (Fibonacci + prime sieve + multi-stream analytics). S41 is a future story; this session only repositions it as the primary demo.
- The 5 quant plugins' bodies (real Binance WS, NewsAPI, InfluxDB v2, MongoDB, yata/ta-lib). S34–S40 in `docs/best-practices/quant/stories.md` are the future work.
- 3-node cluster + e2e SQL deploy (S40).
- HITL seed-user walkthrough.
- The pre-existing warnings in `crates/bee-control/tests/{deploy_pipeline,raft_cluster}.rs`.

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-08-restructure-quant-to-best-practices.md`. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** — execute tasks in this session using `executing-plans`, batch execution with checkpoints

Which approach?
