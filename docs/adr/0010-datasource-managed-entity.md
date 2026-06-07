# 0010: Datasource as a managed entity with `use` syntax and tenant namespace

The Datasource model from ADR-0002 ("a Datasource is a Phase with an Adapter field") is technically correct at the runtime level but lacks the **management semantics** that production deployments need: explicit registration, centralized credential storage, lifecycle control, multi-tenant isolation, and observability independent of any one Pipeline. We add a **management layer** on top of the existing runtime model without changing it. A Datasource becomes a first-class managed entity in Bee, registered by an admin and referenced by Pipeline Authors via the `use` SQL directive.

The new model in SQL:

```sql
use binance;
SELECT * FROM binance.subscribe('BTC/USDT', '1m');
```

> **Naming note**: `binance` is a **documentation example** for a third-party Datasource plugin. Bee core does **not** ship a Binance plugin, a Google News plugin, an InfluxDB plugin, or any other business-specific Datasource. The framework (Adapter trait, `use` syntax, Datasource Registry) is in Bee core; concrete implementations live in separate Plugin crates built as `cdylib`. The only Adapter in Bee core is a generic test fixture (`MockInputAdapter` per stories.md S16) used to verify the mechanism.

Five design rules follow from the user's choices:

1. **`use` is Pipeline-level**, declared at the top of the SQL file, like MySQL's `USE database`. Multiple `use` statements allowed. The scope is the entire compilation unit.
2. **Output Datasources use the same `use` syntax**. `use influxdb; EMIT INTO influxdb('bitcoin.trade') ...` works identically to Input references.
3. **Strict mode only**. Inline Datasource references (e.g., `binance.subscribe('BTC/USDT', '1m', api_key='...')` without a prior `use binance;`) are compile errors. No permissive mode.
4. **Version selection**: `use binance;` defaults to the latest version compatible with the configured SemVer range. `use binance@1.4.2;` or `use binance@^1.0;` provides explicit pinning using the syntax from ADR-0009.
5. **Tenant namespace is a `uint16`**. Every Datasource has `tenant: u16`; every Job has `tenant: u16`; a Job can only `use` Datasources from its own tenant or from tenant `0` (global). MVP enforces the struct field but not the access rule (all tenants `0`); 1.x turns on enforcement.

The runtime layer (ADR-0002, ADR-0003) is unchanged. The Datasource managed entity is a **view on top of** the same `DatasourceSignature = sha256(adapter + config)`. The signature becomes the Datasource's storage identity; the user-visible name is a separate, human-friendly handle. Producer Pipeline sharing (ADR-0003) is automatic — a Datasource is, by definition, a single Producer serving N Subscribers.

## Consequences

- **All credentials live in Bee's secret store, never in SQL**. The Datasource's `config` field references secrets by ID; the Plugin reads them at runtime via the BeeHost API. SQL authors never see or write API keys.
- **Compile-time validation** is now strong: an unknown Datasource name, an adapter that doesn't expose the called method, or a method signature mismatch is a Pipeline submit error, not a runtime surprise.
- **Lifecycle** of a Datasource is independent of any Pipeline. Admin can `pause` a Datasource to trigger Draining on all referencing Jobs, or `test` it to probe connectivity without deploying a Pipeline.
- **Multi-tenancy** is structurally prepared (`u16` namespace fields exist) but not enforced in MVP. Adding enforcement in 1.x is a config change, not a schema change.
- **Datasource management CLI** is the new operational surface: `bee datasource list / create / inspect / test / pause / resume / delete`. These are admin actions; the Pipeline Author side only has `use`.
- **Version conflicts** (two Pipelines pinning the same Datasource to incompatible versions) are detected at submit time; resolution is via Datasource's `version_spec` update.
- **The Datasource is the "Producer"**: the runtime `Producer Pipeline` concept from ADR-0003 becomes an implementation detail. Bee maintains one running instance per Datasource; Subscribers are Jobs that point to it.
- **No backwards-incompatible change to existing ADRs** (ADR-0002 / 0003). The management layer is additive.
