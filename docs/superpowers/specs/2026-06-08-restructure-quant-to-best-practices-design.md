# Restructure · Move S33–S40 (quant) to `best-practices/quant/`, reposition S41 as primary demo

**Date**: 2026-06-08
**Status**: design — pending approval
**Owner**: docs + plugins + stories restructuring (one-shot)
**Story**: cross-cutting (not a single story; multiple stories affected)

## Context

The repo currently foregrounds a quant-trading HITL milestone (S33) + 6 production-grade quant plugins (S34–S40) + 2 quant SQL pipelines + 1 quant demo script. The user wants:

1. The project's primary story to be S41 (performance showcase: Fibonacci + prime sieve + multi-stream analytics) — generic, no domain assumptions.
2. The quant work to be moved to a separate "best practices" documentation section because it's a large chunk of real-world business example, not the project's main story.
3. The 5 quant-flavored "mock" plugins to lose the `mock` suffix (they're scaffolding for future production plugins, not throwaway mocks) and live in `plugins/quant/`.

## Decisions (locked in via brainstorming)

1. **Best-practices location**: new `docs/best-practices/quant/` directory (option A).
2. **5 plugins**: move to `plugins/quant/`, drop the `-mock` suffix from the crate names (e.g., `bee-plugin-binance-mock` → `bee-plugin-binance`).
3. **ADR-0011 + S33 docs**: move to `docs/best-practices/quant/adr/` and `docs/best-practices/quant/specs|plans/`. ADR-0001 to ADR-0010 stay in main `docs/adr/`.
4. **S41 scope**: just restructuring this session. S41 remains a future story (lines 1461+ of `docs/stories.md`); the main repo's primary demo slot will be empty until S41 is implemented in a future session.

## Architecture (the new layout)

```
/Users/shaw/Developer/rust/bee/
├── README.md                                  (UPDATE: point to S41, mention best-practices)
├── Cargo.toml                                 (UPDATE: workspace members paths)
├── docs/
│   ├── architecture.md                       (KEEP; check for quant references → UPDATE if any)
│   ├── internals.md                          (KEEP; check for quant references → UPDATE if any)
│   ├── product-design.md                     (UPDATE: remove Scenario A; promote Scenario D to primary)
│   ├── stories.md                             (UPDATE: remove S33-S40; reposition S41; update dep graph)
│   ├── adr/                                   (KEEP only ADR-0001..0010)
│   │   ├── 0001-...0010-...md                (KEEP — generic decisions)
│   │   └── (ADR-0011 MOVES out)
│   └── superpowers/
│       ├── specs/
│       │   ├── 2026-06-07-s17-stream-signature-design.md   (KEEP — generic infra; light edit for BTC refs)
│       │   └── 2026-06-08-s33-deferred-ffi-design.md       (MOVE)
│       └── plans/
│           ├── 2026-06-07-s17-stream-signature.md           (KEEP)
│           └── 2026-06-08-s33-deferred-ffi.md               (MOVE)
├── crates/                                   (KEEP — generic core)
├── plugins/
│   ├── quant/                                (NEW directory)
│   │   ├── bee-plugin-binance/                (renamed from bee-plugin-binance-mock)
│   │   ├── bee-plugin-google-news/            (renamed from bee-plugin-google-news-mock)
│   │   ├── bee-plugin-influxdb/               (renamed from bee-plugin-influxdb-mock)
│   │   ├── bee-plugin-mongodb/                (renamed from bee-plugin-mongodb-mock)
│   │   └── bee-plugin-ta-lib/                 (renamed from bee-plugin-ta-lib-mock)
│   └── (S41 plugins land here in a future session)
├── examples/                                 (currently empty after the move; S41 lands here in a future session)
├── scripts/                                  (currently empty after the move; S41 lands here in a future session)
├── docs/best-practices/quant/                (NEW directory)
│   ├── README.md                              (NEW: "what is this section" pointer)
│   ├── stories.md                            (NEW: extracted S33 + S34–S40)
│   ├── adr/
│   │   └── 0011-stream-identity-and-backfill.md
│   ├── examples/
│   │   ├── quant_btc_macd.sql
│   │   └── quant_btc_sentiment.sql
│   ├── scripts/
│   │   └── demo-quant-prod.sh
│   ├── specs/
│   │   ├── 2026-06-07-s33-plugin-crates-design.md
│   │   └── 2026-06-08-s33-deferred-ffi-design.md
│   └── plans/
│       └── 2026-06-08-s33-deferred-ffi.md
└── (the 5 plugin crates are now in plugins/quant/, see above)
```

## Files to MOVE (git mv, preserving history)

| Source | Destination |
| --- | --- |
| `docs/superpowers/specs/2026-06-08-s33-deferred-ffi-design.md` | `docs/best-practices/quant/specs/` |
| `docs/superpowers/plans/2026-06-08-s33-deferred-ffi.md` | `docs/best-practices/quant/plans/` |
| `docs/adr/0011-stream-identity-and-backfill.md` | `docs/best-practices/quant/adr/` |
| `examples/quant_btc_macd.sql` | `docs/best-practices/quant/examples/` |
| `examples/quant_btc_sentiment.sql` | `docs/best-practices/quant/examples/` |
| `scripts/demo-quant-prod.sh` | `docs/best-practices/quant/scripts/` |
| `plugins/bee-plugin-binance-mock/` (whole dir) | `plugins/quant/bee-plugin-binance/` |
| `plugins/bee-plugin-google-news-mock/` | `plugins/quant/bee-plugin-google-news/` |
| `plugins/bee-plugin-influxdb-mock/` | `plugins/quant/bee-plugin-influxdb/` |
| `plugins/bee-plugin-mongodb-mock/` | `plugins/quant/bee-plugin-mongodb/` |
| `plugins/bee-plugin-ta-lib-mock/` | `plugins/quant/bee-plugin-ta-lib/` |

## Files to CREATE (new)

| Path | Content |
| --- | --- |
| `docs/best-practices/quant/README.md` | Short pointer: "this section is the quant trading reference implementation; main repo is the generic S41 performance showcase. See stories.md for S33–S40 and adr/ for ADR-0011." |
| `docs/best-practices/quant/stories.md` | Extract from main `docs/stories.md`: §S33 (quant HITL milestone) + §S34–§S40 (the 6 production plugins + e2e deploy). The text is moved verbatim; cross-references to other stories are updated to point to the main repo. |

## Files to UPDATE in place

### `docs/stories.md`
- **Remove** the lines referring to S33–S40 in the dependency graph (lines 54–62, 81).
- **Remove** the "H. Quant trading spike (prod)" row in the parallel-paths table (line 81).
- **Remove** the S33 + S34–S40 sections (the body content, not the section headers — we move the body to `docs/best-practices/quant/stories.md`).
- **Remove** the S33 entry from "Key milestones" (line 92).
- **Reposition** S41: move the §S41 header + body to immediately after the "7 parallel paths after S10" table (it's the primary demo, not just a story). Add a "Next demo" callout in the "Key milestones" section.
- **Update** the story count footnote (line 76 area) — was "32 stories + S33 spike" → becomes "32 stories + S41 performance showcase" (S33-S40 live in best-practices/quant/).
- **Light edit** to remove `binance` / `google-news` / `influxdb` / `mongodb` / `ta-lib` from the §S17 acceptance criteria (replace with "PluginKind::Plugin(...)" generic references or similar).

### `docs/product-design.md`
- **Remove** "### Scenario A: Quant decision pipeline" (lines 103–122).
- **Reorder** Scenario D (Performance showcase) to be the first scenario (rename to "Scenario A: Performance showcase (the 5-minute evaluator demo)").
- **Update** the "What's in the repo" section: replace "5 mock plugins" / "6 production plugins" with "the core crates under `crates/`, plus the `plugins/quant/` reference implementations" + "see `docs/best-practices/quant/` for the quant trading reference".
- **Update** the "Roadmap" / milestones to point to S41 as the next demo.

### `README.md`
- **Update** the "Quickstart" section: instead of pointing to `scripts/demo-quant-prod.sh` and 4 mock plugins, point to the future S41 demo (one-liner: "the canonical 5-minute end-to-end demo is `examples/performance/` (S41, in flight)").
- **Update** the docs table: `docs/stories.md` is now "33 implementation stories (S0–S32 + S41 spike)" with a footnote pointing to `docs/best-practices/quant/stories.md` for S33–S40.
- **Update** the example plugins list: drop the 5 quant-named ones from the main repo, mention they're in `plugins/quant/`.
- **Update** the demo-script reference: replace `scripts/demo-quant-prod.sh` (now in best-practices) with the future S41 demo.
- **Update** the architecture diagram (if any) to reflect the new layout.

### `Cargo.toml` (workspace root)
- **Update** the `members` list:
  - Drop `plugins/bee-plugin-binance-mock`, `plugins/bee-plugin-google-news-mock`, `plugins/bee-plugin-influxdb-mock`, `plugins/bee-plugin-mongodb-mock`, `plugins/bee-plugin-ta-lib-mock`.
  - Add `plugins/quant/bee-plugin-binance`, `plugins/quant/bee-plugin-google-news`, `plugins/quant/bee-plugin-influxdb`, `plugins/quant/bee-plugin-mongodb`, `plugins/quant/bee-plugin-ta-lib`.

### `docs/superpowers/specs/2026-06-07-s17-stream-signature-design.md` + plan
- **Light edit**: the §1 "Decision" + §2 "Acceptance criteria" mention `binance.subscribe('BTC/USDT', '5min')` as the canonical example. The S17 design is generic infrastructure; the example is fine but the "Backfill" §2 reference should point to the moved ADR-0011 location.
- Update the ADR-0011 link to point to `../../best-practices/quant/adr/0011-stream-identity-and-backfill.md` (or a docs-relative path).

### `crates/bee-plugin-sdk/Cargo.toml`, `crates/bee-registry/Cargo.toml`, `crates/bee-runtime/Cargo.toml`, `crates/bee-control/Cargo.toml`
- **Possibly no change needed**: the SDK/registry/runtime/control code doesn't import the plugin crates by name; it uses the FFI vtable interface. Verify with `grep -r "bee-plugin" crates/`. If any crate hard-references a plugin name, fix the path.

### `docs/architecture.md`, `docs/internals.md`
- **Possibly no change needed**: these are generic architecture docs. Verify with a quick grep for "binance" / "quant" / "BTC". If any mention, light edit.

## Plugin renames (5 plugins)

For each plugin, the move involves:
1. `git mv plugins/bee-plugin-{name}-mock plugins/quant/bee-plugin-{name}`
2. Edit `Cargo.toml`: change `name = "bee-plugin-{name}-mock"` → `name = "bee-plugin-{name}"`. Update the description to drop "mock" / "sine-wave" / "test fixture" language — describe it as a real reference implementation.
3. Edit `src/lib.rs`:
   - Drop the `cdylib_plugin!` mock-related comments
   - Update the crate-level doc: it's a reference plugin for the quant domain (binance = real-time price stream; influxdb = real-time TSDB sink; etc.). Note that the underlying data source is currently a sine-wave / log-file mock, but the plugin STRUCTURE is production-grade.
   - Drop "mock" / "sine-wave" from test names (e.g., `vtable_next_returns_sine_wave_event` → `vtable_next_returns_tick_event`).
4. The cdylib artifact renames automatically (the workspace uses `name` from Cargo.toml): `libbee_plugin_binance_mock.dylib` → `libbee_plugin_binance.dylib`.
5. The `bee plugin list` subcommand (in `bee/src/main.rs`) needs its hardcoded list of plugins updated: `binance-mock` → `binance`, etc. (5 places).

## Commit strategy

**Single commit** (per the S17 + S33-deferred consolidation precedent):

```bash
git reset --soft <ref-before-restructure>  # i.e., `3d16622^`
# After this, all the moves + edits are in the index.
git commit -m "Restructure: move S33-S40 to best-practices/quant/, rename plugins, S41 becomes primary demo

[long commit body describing the 3 sections above]
"
```

The reference for "ref-before-restructure" is `3d16622^` (the S33-deferred commit's parent, which is the design commit). All work in the S33-deferred commit plus this restructuring lands in the single commit.

## Acceptance criteria

- [ ] `cargo build --workspace` clean (0 new warnings beyond pre-existing)
- [ ] `cargo test --workspace` all green; the 354 tests still pass with the renamed plugins
- [ ] `git ls-files | grep -E "(binance|google-news|influxdb|mongodb|ta-lib)"` shows the 5 plugins only under `plugins/quant/` (not under `plugins/` directly)
- [ ] `docs/best-practices/quant/` exists with: README, stories.md, adr/, examples/, scripts/, specs/, plans/
- [ ] `docs/adr/` contains only ADR-0001 through ADR-0010 (no 0011)
- [ ] `docs/stories.md` does not contain "binance" / "google-news" / "influxdb" / "mongodb" / "ta-lib" / "quant" (except in the "see best-practices/quant" footnote)
- [ ] `docs/product-design.md` does not contain "Scenario A: Quant"
- [ ] `examples/` is empty (or contains only S41 files if/when they land)
- [ ] `scripts/` is empty (or contains only S41 files if/when they land)
- [ ] The 5 plugins' `Cargo.toml` `name` field has no `-mock` suffix
- [ ] The 5 plugins' cdylib artifacts are named without `_mock`
- [ ] The `bee plugin list` subcommand lists the 5 renamed plugins

## Risks

1. **Plugin rename breaks `bee plugin list`**: the CLI subcommand hardcodes the 5 plugin names (added in the S33-deferred commit). Need to update 5 places in `bee/src/main.rs`. **Mitigation**: grep for `bee-plugin-` in `bee/src/main.rs`; update each match.

2. **Test name changes break `grep`-based test discovery**: existing test names mention "mock" (e.g., `vtable_next_returns_sine_wave_event`). The user wants to drop "mock" from test names. The test bodies are unchanged; only the function names change. **Mitigation**: search the codebase for test names containing "mock" and update; verify tests still pass with the new names.

3. **S17 design/plan still mentions BTC/binance**: the S17 design is generic but uses binance.subscribe in examples. The user said "清除...文章" (clear articles). Strictly, the S17 design is a "core" article. **Mitigation**: light edit to replace `binance.subscribe('BTC/USDT', '5min')` with `PluginKind::Plugin(\"exchange\", \"subscribe\")` (generic) in the §2 acceptance criteria; leave the §1 "Decision" formula unchanged (it's pure code, no domain reference).

4. **ADR-0011 cross-reference breakage**: ADR-0003, ADR-0010 (which stay in main) might reference ADR-0011. Verify and update the links. **Mitigation**: grep `docs/adr/0003-...0010` for "0011" and update the links.

5. **Workspace `Cargo.toml` glob conflict**: `plugins/*` vs `plugins/quant/*` — both patterns could match. We use explicit listing (not globs) for the 5 plugins, so no conflict. **Mitigation**: keep explicit listing; document in the commit body.

6. **Single-commit consolidation risk**: `git reset --soft` squashes the S33-deferred work AND the restructuring into one commit. If anything goes wrong mid-reset, recovery is hard. **Mitigation**: do the moves + edits in the working tree first (not via `git reset --soft`); only do the soft-reset as the final step. If anything goes wrong, we have a working tree to recover from.

## Implementation order

1. **Read all the files** we'll touch (5 plugin `Cargo.toml` + 5 plugin `src/lib.rs`; `docs/stories.md`; `docs/product-design.md`; `README.md`; workspace `Cargo.toml`; `bee/src/main.rs`).
2. **Move** the 5 plugins via `git mv` (preserves history).
3. **Edit** the 5 plugins' `Cargo.toml` + `src/lib.rs` (rename + drop "mock" from test names).
4. **Edit** `Cargo.toml` (workspace) to point to `plugins/quant/`.
5. **Update** `bee/src/main.rs` to use the renamed plugin names in the `bee plugin list` subcommand.
6. **Verify** `cargo build --workspace` + `cargo test --workspace` (should still be 354 passing tests).
7. **Move** the docs (ADR-0011, S33 docs, S33-deferred design/plan) via `git mv` to `docs/best-practices/quant/`.
8. **Move** `examples/quant_*.sql` and `scripts/demo-quant-prod.sh` via `git mv` to `docs/best-practices/quant/`.
9. **Create** `docs/best-practices/quant/README.md` and `docs/best-practices/quant/stories.md` (extract the S33 + S34–S40 sections from main `docs/stories.md`).
10. **Update** `docs/stories.md` (remove S33–S40, reposition S41, update dep graph + table).
11. **Update** `docs/product-design.md` (remove Scenario A, promote Scenario D to primary, update §"What's in the repo").
12. **Update** `README.md` (point to S41 future demo, mention best-practices/quant/).
13. **Light edit** `docs/superpowers/specs/2026-06-07-s17-stream-signature-design.md` and the matching plan to drop BTC/binance examples (replace with generic `PluginKind::Plugin(...)` in acceptance criteria; update ADR-0011 cross-reference).
14. **Final verify**: `cargo test --workspace` + `cargo build --workspace` + the structural greps in the acceptance criteria.
15. **Single commit** (per the consolidation precedent): `git reset --soft 3d16622^` (the S33-deferred commit's parent), then commit with the long commit body.

## Out of scope

- Implementing S41 (Fibonacci + prime sieve + multi-stream analytics). S41 stays as a future story; this session only restructures the project around it.
- The 5 quant plugins' bodies (the sine-wave generators, the log-file sinks). They stay as placeholders; S34–S40 (in `docs/best-practices/quant/stories.md`) will replace them with real Binance WS / NewsAPI / etc. in future sessions.
- 3-node cluster + e2e SQL deploy (S40 in best-practices/quant/stories.md).
- HITL seed-user walkthrough.
- Removing the pre-existing warnings in `bee-control/tests/{deploy_pipeline,raft_cluster}.rs` (out of scope; pre-date this commit).
