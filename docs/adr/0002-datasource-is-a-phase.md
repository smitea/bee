# 0002: Datasource is a Phase with an Adapter

The original sketch treats Datasource as a first-class object ("Datasource consists of Input and Output") with its own identity, lifecycle, and Registry entry, parallel to Phase. This forks the runtime into two scheduling paths and forces the control plane to track two kinds of ownership. We unify: a Datasource IS a Phase whose `adapter` field names an Input or Output plugin. Lifecycle (Pending → … → Orphaned / Migrating), Work-Stealing, and Raft registration are all inherited unchanged. The Datasource name is kept for the user-facing concept (and for SQL syntax like `binance.subscribe(...)`), but at runtime it is just a Phase with an `Adapter` reference. The version/config of the external connection lives on the Adapter plugin (which has its own upgrade lifecycle via Plugin Manager), not on a Datasource instance.

## Consequences

- Plugin Manager exposes Adapters, not Datasources. Upgrading an Adapter upgrades every Datasource Phase that references it.
- Multiple Pipelines that all reference the same Adapter+config share one **Producer Pipeline** (a single-Phase Pipeline Job that runs the Adapter); subscribers reuse its stream. This is the rate-limit-friendly sharing path: 5 Pipelines using `binance.subscribe('BTC/USDT', '5min')` cost one network connection, not five.
- Cross-Pipeline edges and Datasource-as-Pipeline are the same mechanism: both are DAG edges that happen to cross Job boundaries and are served by the BRP data channel.
- We lose the ability to give "the Datasource" a separate ID, but we keep all the user-visible behavior: install / upgrade / reload, hot plugin swap, observability, and the `EMIT INTO` / `binance.subscribe` SQL syntax.
