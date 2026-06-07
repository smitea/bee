# 0003: Producer Pipeline pattern for rate-limited Datasource sharing

When multiple Pipeline Jobs reference the same rate-limited external source (e.g., Binance API), spinning up one network connection per Job is wasteful and may exceed the source's rate limit. We unify the sharing path with the existing Cross-Pipeline Edge mechanism: the first Job to reference a given Datasource signature (`hash(AdapterId + config_payload)`) is deployed as a **Producer Pipeline** — a single-Phase Pipeline Job that runs the Input Adapter. Subsequent Jobs with matching Datasource signatures detect the existing Producer in the Raft state and degrade their Datasource Phase into a subscription edge, reading the stream from the Producer over BRP. The Producer owns the rate-limited network connection; subscribers reuse the stream. Failover is unified: when the Producer Node fails, all subscribers enter `Waiting for Upstream` and reconnect after the Producer migrates to a new Node.

## Consequences

- Sharing is automatic — no explicit user declaration. The compiler + Control Plane decide based on Datasource signature.
- Subscribers cannot directly tune Adapter config (it is owned by the Producer). Version upgrades are coordinated by upgrading the Producer first; subscribers consume the new stream transparently.
- Subscriber failover is coupled to Producer failover: subscribers can only resume when the Producer is back.
- If the Producer is intentionally taken down (e.g., for Adapter upgrade), all subscribers experience a coordinated pause; this is the price of paying for one network connection instead of N.
- Backpressure flows naturally: a slow subscriber slows the Producer's consumption, which back-pressures the upstream (the rate-limited source).
- **Refined by [ADR-0010](./0010-datasource-managed-entity.md)**: the `DatasourceSignature` is more precisely a `StreamSignature = sha256(datasource_name || adapter_method || canonicalized_call_args)`. Two Pipelines calling `binance.subscribe('BTC/USDT', '5min')` and `binance.subscribe('ETH/USDT', '5min')` get **different** Producers (different streams), even though they share the same Datasource config (same API key).
