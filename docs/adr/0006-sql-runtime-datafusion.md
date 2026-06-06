# 0006: SQL Runtime — DataFusion with extensions; Lua and other DSLs deferred to 1.x

Bee's SQL runtime is built on Apache Arrow DataFusion. DataFusion provides SQL parsing, planning, and execution in Rust with Apache 2.0 licensing, vectorized columnar execution, and a rich UDF extension mechanism. We extend it with: (1) `ASOF JOIN` as a new `JoinKind` (essential for the financial time-series use case the user described), (2) `EMIT INTO` as a new top-level statement type that drives a continuous query, and (3) custom UDFs for `MACD / EMA / KRONOS / decision_tree / sentiment_analyzer`. Continuous queries are driven by a micro-batch executor with a configurable window (default 1 second; can be tightened to 10ms for quant scenarios). DataFusion's optimizer extension points (`OptimizerConfig`, `PhysicalOptimizerRule`, SQL hints) are exposed in Bee's Pipeline configuration so users can override default rules, hint the planner, and tune cost models — required for the millisecond-level quant adaptation the user requested.

Lua runtime and other DSLs (Python, JSON, YAML) are **explicitly deferred to 1.x** per the user's Q6. MVP supports SQL only; the `bee-dsl-lua` crate is removed from the 0.x roadmap.

## Consequences

- **Performance**: DataFusion per-event query overhead is ~100µs-1ms (planning, Arrow conversion, executor init). For millisecond-level quant, the micro-batch window must be tight (≤10ms) and per-event mode (no batching) is available as a special case. The in-memory KV hot cache (default in MVP per ADR-0004) is critical for ms-level state access.
- **Optimizer control**: Bee's Pipeline config exposes DataFusion's optimizer extension points. Users can disable rules, add custom `PhysicalOptimizerRule`s, use SQL hints (e.g., `/*+ JOIN_ORDER(a, b) */`), and tune cost-model weights. This is the mechanism that addresses the user's "control over SQL optimization" requirement.
- **`ASOF JOIN`**: New `JoinKind` added to DataFusion (or forked if upstream PR is not yet merged). Cost is similar to other joins for typical window sizes; supports both equi-time and inequality matchers.
- **`EMIT INTO`**: Top-level statement that triggers continuous-query mode. Compiles to a Sink node + a micro-batch executor loop with the configured window.
- **Lua deferred**: `bee-dsl-lua` is not in the 0.x roadmap. SQL is the only DSL in MVP. Users who need Lua must wait for 1.x or write a Rust plugin that calls mlua internally.
- **Other DSLs deferred**: Python (PyO3), JSON, YAML DSLs are not in MVP. The Pipeline Author API is a Rust SDK that wraps the SQL compiler.
