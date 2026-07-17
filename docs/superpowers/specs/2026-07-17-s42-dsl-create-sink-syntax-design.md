# S42 — DSL `CREATE SINK` Syntax (sink-via-plugin)

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S15 (DataFusion executor wrapper)
**ADRs:** 0006 (SQL runtime), 0010 (per-call args)
**Status:** Draft (pending review)
**Source WIP:** `stash@{0}` — `crates/bee-dsl-sql/src/{lib,physical,preprocess}.rs`

## Why this story exists

The current `EMIT INTO <target>` preprocessor recognises only `Console` (built-in stdout). Real Pipelines want to send rows to plugin-backed sinks (`influxdb`, `mongodb`, `binance` via the SINK direction, …) without writing the same `EMIT INTO <plugin>` boilerplate on every query.

This story adds a declarative **`CREATE SINK <name> AS <body>`** statement that binds a `<body>` (a SELECT) to a named plugin output target. The body evaluates once; its rows route to `<name>`'s OutputAdapter.

## Stash WIP starting point

`stash@{0}` already contains a partial implementation:

| File | WIP status |
|---|---|
| `crates/bee-dsl-sql/src/preprocess.rs` | adds `EmitTarget::Plugin(String)` variant; extends `strip_emit_into` to recognise non-`console` targets as plugin; adds `CreateKind::Sink` variant + `strip_create_sink` function; extends `find_create_statement` to recognise `CREATE SINK` |
| `crates/bee-dsl-sql/src/lib.rs` | wires `strip_create_sink` into `preprocess_sql_v2` |
| `crates/bee-dsl-sql/src/physical.rs` | matches `EmitTarget::Plugin(name)` in `run_pipeline_with_config` (placeholder: prints "emitted N rows to sink `name`") |

The stash WIP has three gaps that this story closes:

1. **No EMIT-INTO injection.** The stash's `strip_create_sink` substitutes `CREATE SINK foo AS <body>` with just `<body>`. The sink routing never fires because there's no `EMIT INTO foo` to trigger it. **This story adds the injection** so the SINK body evaluates and its rows route via the existing `EmitTarget::Plugin` arm.
2. **No unit tests** for the `Plugin` variant of `EmitTarget` or for the `Sink` variant of `CreateKind`. The stash just adds the code.
3. **No strict-mode enforcement** linking the SINK target name to a registered Datasource or plugin. Without that, `CREATE SINK foo AS ...` for an unknown `foo` silently desugars; the runtime emits "to sink `foo`" with no validation.

## Approach (chosen): desugar CREATE SINK → body + EMIT INTO

**The rule**: `CREATE SINK <name> AS <body>` is rewritten by the preprocessor to `<body>` followed by `EMIT INTO <name>`. Everything else (parsing, validation, runtime routing) reuses the existing EMIT INTO machinery.

### Why this approach

| Alternative | Why rejected |
|---|---|
| **A. Plugin reuses existing Datasource mechanism** (treat the SINK name as a `Datasource` reference + use S29 strict-mode `use <name>;` to validate) | Conceptually wrong — `influxdb` as a SOURCE pulls data from InfluxDB; as a SINK, it writes to InfluxDB. Two distinct roles, one Datasource registration. S29 already disambiguates by Adapter direction (Input vs Output). |
| **B. Separate Sink registry** (parallel to Datasource registry, new `Sink { name, plugin_id, config }` map) | Adds a new entity class + lifecycle + RBAC + ACL story. Too much for the MVP. The plugin's OutputAdapter is already registered via `register_vtable!`; the SINK just needs to point at one. |
| **C. Desugar CREATE SINK → body + EMIT INTO** (chosen) | Zero new runtime plumbing. The SINK name is just a sugar for an EMIT INTO. Plugin validation reuses the existing `EmitTarget::Plugin` arm. Future evolution (per-Adapter OutputAdapters, plugin-config routing) can extend the EMIT INTO arm without touching the SINK syntax. |

### What the preprocessor emits

Given input:

```sql
use binance;
use influxdb;

CREATE SINK influxdb AS
SELECT close, volume FROM binance.subscribe('BTC/USDT', '5min')
WHERE close > 0;
```

The preprocessor rewrites to:

```sql
use binance;
use influxdb;

SELECT close, volume FROM binance.subscribe('BTC/USDT', '5min')
WHERE close > 0;
EMIT INTO influxdb
```

(DataFusion parses the rewritten form. The pipeline's downstream `strip_emit_into` strips `EMIT INTO influxdb` and stores `EmitTarget::Plugin("influxdb")` in the pipeline state. `physical::run_pipeline_with_config` matches the `Plugin(name)` arm.)

### Multi-statement rules

| Scenario | Behavior |
|---|---|
| One `CREATE SINK`, one `<body>` | The body is the SELECT; the SINK binds it. The body's rows route to the sink. |
| `CREATE SINK foo AS <body1>; CREATE SINK bar AS <body2>;` (two SINKs) | **MVP**: error. The MVP supports exactly one SINK per SQL file. Multi-sink is a follow-up (see "Out of scope"). |
| `CREATE SINK foo AS <body>` + `EMIT INTO console` (two outputs) | **MVP**: error. Same MVP constraint — one sink target per query. |
| `<body>` containing `EMIT INTO console` itself | The body's `EMIT INTO` is itself stripped before SINK desugaring; effectively two outputs. Same MVP error. |

### Strict-mode validation (S29 reuse)

`S29` enforces that any `binance.subscribe(...)` call must be preceded by `use binance;`. The SINK target name follows the same rule:

- If `CREATE SINK binance AS ...` appears without prior `use binance;`, that's a compile error.
- The check is a new arm in `preprocess::check_strict_mode` that mirrors the existing source-side check, but on `EmitTarget::Plugin`.

### Real plugin routing (deferred)

For MVP, the `EmitTarget::Plugin(name)` arm in `physical::run_pipeline_with_config` (already in the stash) prints a placeholder: `(emitted N row(s) to sink <name>)`. The actual cross-FFI dispatch to the plugin's OutputAdapter is **out of scope** for S42 — it lives in a future story (S42.x or later).

The placeholder is honest: it compiles, it runs, the SELECT evaluates, but the rows don't cross the FFI to the plugin yet. The user sees the row count and the sink name. This is the same MVP posture as `EmitTarget::Console` did before S41 added the console writer.

## Scope

### In scope

1. **Desugar `CREATE SINK <name> AS <body>` → `<body>` + `EMIT INTO <name>`** in `preprocess_sql_v2`
2. **One SINK per SQL** (MVP): error on multiple SINKs or on a SINK + `EMIT INTO` combo
3. **Strict-mode check** for `use <name>;` before `CREATE SINK <name>` (same shape as source-side check)
4. **`strip_create_sink` refinement**: return `(Some(name), rewritten_sql)` where `rewritten_sql` includes the body + `EMIT INTO <name>` appended
5. **Unit tests** (5 new + a refresh of the existing strip_create tests):
   - `find_create_statement` recognises `CREATE SINK foo AS SELECT ...`
   - `strip_create_sink` returns `(Some("foo"), "SELECT ...\nEMIT INTO foo\n")` for a single-SINK SQL
   - `strip_create_sink` returns `(None, original_sql)` when no SINK is present
   - `strip_create_sink` returns an error / `(None, original_sql)` (we'll decide one; see Decision Matrix) on multiple SINKs
   - `check_strict_mode` rejects `CREATE SINK <unknown>` (no matching `use`)
   - `check_strict_mode` accepts `CREATE SINK <known>` (matching `use` exists)
6. **CLI sanity**: `bee run` accepts a SQL with `CREATE SINK` and runs the body via the existing pipeline

### Out of scope (deferred)

- **Real cross-FFI dispatch** to the plugin's OutputAdapter vtable (rows actually crossing FFI). This is a S42.x or S49 follow-up. The MVP prints a placeholder.
- **Per-row emit** — the MVP runs the body once per pipeline start. For now, the pipeline is single-pass.
- **Per-call args for SINK** (e.g., `CREATE SINK influxdb WITH bucket = 'archive' AS ...`). The current design treats the sink name as the only sink config; richer config is a S42.x.
- **Multi-sink per SQL** — explicitly deferred. A future story can revisit this if needed.
- **`EMIT INTO <plugin>` legacy syntax** — keep working (no breaking change). The stash's `strip_emit_into` already recognises non-console targets; S42 just ensures the strict-mode check covers them.

## File structure

| File | Action |
|---|---|
| `crates/bee-dsl-sql/src/preprocess.rs` | Modify: refine `strip_create_sink` to append `EMIT INTO <name>` + add strict-mode arm |
| `crates/bee-dsl-sql/src/preprocess.rs::tests` | Modify: 5 new tests + refreshed tests |
| `crates/bee-dsl-sql/src/lib.rs` | Modify: (the stash already wires this; verify + ship) |
| `crates/bee-dsl-sql/src/physical.rs` | No change (the stash's Plugin arm prints placeholder — that IS the MVP) |
| `docs/product-design.md` | Modify: 1 paragraph in §4.5 "SQL DSL" describing `CREATE SINK` |
| `CONTEXT.md` | Optional: note the new syntax in the Pipeline section |

## Acceptance criteria

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` ≥ 415 passed, 0 failed
- [ ] `cargo test -p bee-dsl-sql` — new + refreshed unit tests all pass:
  - `find_create_statement_recognises_create_sink`
  - `strip_create_sink_extracts_name_and_body`
  - `strip_create_sink_appends_emit_into_target`
  - `strip_create_sink_returns_none_when_no_sink`
  - `check_strict_mode_rejects_create_sink_without_use`
  - `check_strict_mode_accepts_create_sink_with_use`
- [ ] SQL with `CREATE SINK foo AS SELECT * FROM bar` compiles via `bee run` and prints `(emitted N row(s) to sink foo)` (placeholder, per "Real plugin routing (deferred)")
- [ ] SQL with multiple `CREATE SINK foo AS ...; CREATE SINK bar AS ...;` returns a clear compile error (multi-sink not supported in MVP)
- [ ] SQL with `CREATE SINK unknown_plugin AS ...` (no `use unknown_plugin;`) returns a clear strict-mode error
- [ ] Stash diff `git stash show stash@{0} -- crates/bee-dsl-sql/` applied on top of HEAD with no merge conflicts (after stash drop)
- [ ] No `*.sql` change in `examples/performance/` (the demo SQLs do not use SINK in MVP)

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `CREATE SINK <name> AS <body>` desugars to `<body>` + `EMIT INTO <name>` | ✓ (S42 + tests) | N |
| One-SINK-per-SQL MVP constraint | ✓ | N — multi-sink is a follow-up |
| Strict-mode `use <name>;` enforcement on SINK target | ✓ | N — ACL is 1.x |
| Real cross-FFI dispatch to OutputAdapter vtable | — | N — deferred to S42.x / S49 |
| Plugin-side sink config (`WITH bucket = 'archive' AS ...`) | — | N — deferred |

## Related work

- **S29** (Datasource managed entity + `use` strict mode) — the strict-mode check for SINK reuses S29's existing `use <name>;` validation
- **S17** (Producer Pipeline mode + StreamSignature) — the runtime side; S42's MVP placeholder doesn't actually create a Producer
- **S43** (Plugin KV port) — independent; S42 doesn't depend on it
- **S44** (S41 demo cleanup) — independent; S42 doesn't touch `examples/performance/`
- **S45** (`.gitignore` cleanup) — independent

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| Multi-sink in MVP? | **No** — error | Keeps the preprocessor + dispatch logic simple; future story can revisit |
| Real plugin dispatch in MVP? | **No** — placeholder | The runtime-side routing lives in a separate story (S42.x) |
| Legacy `EMIT INTO <plugin>` syntax? | **Keep working** | Strict-mode applies to it too; no breaking change |
| `WITH <args>` clause on SINK? | **Defer** | Treats SINK name as the only sink config for now |

If any of these decisions should change, the user can override during the spec review.
