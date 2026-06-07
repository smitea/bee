# 0010: Datasource as a managed Provider with `use` syntax and tenant namespace

The Datasource model from ADR-0002 ("a Datasource is a Phase with an Adapter field") is technically correct at the runtime level but lacks the **management semantics** that production deployments need: explicit registration, centralized credential storage, lifecycle control, multi-tenant isolation, and observability independent of any one Pipeline. We add a **management layer** on top of the existing runtime model without changing it.

**Key distinction (the central design choice)**: a Datasource is a **Provider** — a managed connection to an external system — and is **separate from the Stream** that the Pipeline selects at call time. The Datasource carries connection-level configuration (credentials, base URL, rate-limit settings, which Adapter to use); the SQL call site carries **per-call arguments** that select a specific stream from within that Provider (e.g., symbol, interval, query string).

Concretely:

```sql
use binance;                                          -- references the Datasource "binance" (the Provider)
SELECT * FROM binance.subscribe('BTC/USDT', '5min');  -- "subscribe" is a method on the Adapter the Datasource wraps;
                                                       -- ('BTC/USDT', '5min') are per-call args selecting the stream
```

A Pipeline that wants a different stream of the same Provider just changes the arguments; no Datasource re-registration needed. Conversely, two different Pipelines that call `binance.subscribe('BTC/USDT', '5min')` with identical args **share one Producer** (per ADR-0003, refined — see "Producer Pipeline refinement" below).

Five design rules follow from the user's choices:

1. **`use` is Pipeline-level**, declared at the top of the SQL file, like MySQL's `USE database`. Multiple `use` statements allowed. The scope is the entire compilation unit.
2. **Output Datasources use the same `use` syntax**. `use influxdb; EMIT INTO influxdb.emit('bitcoin.trade', ...)` works identically to Input references.
3. **Strict mode only**. Inline Datasource references (e.g., `binance.subscribe('BTC/USDT', '5min', api_key='...')` without a prior `use binance;`) are compile errors. No permissive mode.
4. **Version selection**: `use binance;` defaults to the latest version compatible with the configured SemVer range. `use binance@1.4.2;` or `use binance@^1.0;` provides explicit pinning using the syntax from ADR-0009.
5. **Tenant namespace is a `uint16`**. Every Datasource has `tenant: u16`; every Job has `tenant: u16`; a Job can only `use` Datasources from its own tenant or from tenant `0` (global). MVP enforces the struct field but not the access rule (all tenants `0`); 1.x turns on enforcement.

### Producer Pipeline refinement (extends ADR-0003)

ADR-0003 defined `DatasourceSignature = sha256(adapter_id + config_payload)` to identify a sharable Producer. With the Provider / Stream separation, the signature is **refined** to be the **StreamSignature**:

```
StreamSignature = sha256(
    datasource_name           -- which Provider
  || adapter_method            -- which method (subscribe, search, emit, ...)
  || canonicalized_call_args   -- the per-call argument list (symbol, interval, etc.)
)
```

Two Pipelines with the same StreamSignature share a Producer. Different symbols, different intervals, or different methods produce different signatures and different Producers — even when they reference the same Datasource (Provider).

## Consequences

- **All credentials live in Bee's secret store, never in SQL**. The Datasource's `config` field references secrets by ID; the Plugin reads them at runtime via the BeeHost API. SQL authors never see or write API keys. Per-call args (symbol, interval, query) are NOT secrets and may appear in SQL.
- **Compile-time validation** is strong: an unknown Datasource name, an Adapter that doesn't expose the called method, or a method signature mismatch is a Pipeline submit error, not a runtime surprise.
- **Lifecycle** of a Datasource is independent of any Pipeline. Admin can `pause` a Datasource to trigger Draining on all referencing Jobs, or `test` it to probe connectivity without deploying a Pipeline.
- **Multi-tenancy** is structurally prepared (`u16` namespace fields exist) but not enforced in MVP. Adding enforcement in 1.x is a config change, not a schema change.
- **Datasource management CLI** is the new operational surface: `bee datasource list / create / inspect / test / pause / resume / delete`. These are admin actions; the Pipeline Author side only has `use`.
- **Version conflicts** (two Pipelines pinning the same Datasource to incompatible versions) are detected at submit time; resolution is via Datasource's `version_spec` update.
- **Sharing granularity** is now correctly per-Stream, not per-Provider. `binance.subscribe('BTC/USDT', '5min')` and `binance.subscribe('ETH/USDT', '5min')` get different Producers (correct — different data), even though they share the same Datasource config (the API key is reused).
- **No backwards-incompatible change to existing ADRs** (ADR-0002 / 0003). ADR-0003's DatasourceSignature is *refined* to StreamSignature; the underlying mechanism (one running Producer per signature, N Subscribers reading its stream) is unchanged.
- **Bee core is business-agnostic**. The Datasource and Adapter abstractions are general; the specific Datasource `binance` and the specific method `subscribe` are illustrative examples for a third-party plugin. No `binance` code is in Bee core.

> **Naming note**: `binance` is a **documentation example** for a third-party Datasource plugin. Bee core does **not** ship a Binance plugin, a Google News plugin, an InfluxDB plugin, or any other business-specific Datasource. The framework (Adapter trait, `use` syntax, Datasource Registry) is in Bee core; concrete implementations live in separate Plugin crates built as `cdylib`. The only Adapter in Bee core is a generic test fixture (`MockInputAdapter` per stories.md S16) used to verify the mechanism.
