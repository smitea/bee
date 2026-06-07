<div align="center">

# 🐝 Bee

**A Rust-native distributed dataflow pipeline compute service for low-latency stream pipelines**

`tokio` · `bincode` · Custom BRP binary protocol

</div>

---

## Project Status

> [!WARNING]
> **Early design stage (pre-alpha).** This repo currently ships design documents only. No crates, source, or binaries are published yet. APIs and crate boundaries are subject to change.

## Documentation Map

| Document | Content | Audience |
| --- | --- | --- |
| [README.md](README.md) | This file: project pitch + roadmap | Everyone |
| [CONTEXT.md](CONTEXT.md) | Domain glossary (Data Plane / Control Plane / Pipeline / Job / Task / …) | Everyone |
| [docs/product-design.md](docs/product-design.md) | Product vision / users / scenarios / business model | Product / early users / partners |
| [**docs/architecture.md**](docs/architecture.md) | **System architecture (OSS format)**: Why / Goals / Principles / Subsystems / Operations / Security / Performance | Evaluators / operators / contributors |
| [**docs/internals.md**](docs/internals.md) | **Implementation details**: BRP wire format / 8-crate boundary / Plugin SDK contract / KV internal layout | Implementers / advanced contributors |
| [docs/stories.md](docs/stories.md) | **32 implementation stories + S33 quant spike** (vertical slice breakdown) | Implementers |
| [docs/adr/](docs/adr/) | Irreversible architecture decision records (ADR) | Implementers |

## Quickstart (after implementation)

> Once the first AFK stories land, the canonical "5-minute end-to-end demo" lives at:
>
> - `scripts/demo-quant.sh` — starts a 3-node cluster, loads 4+ mock plugins, registers Datasources, deploys two quant strategies, verifies Producer sharing, asserts all 10 ADRs' Consequences
> - `examples/quant_btc_strategy.sql` — the canonical quant pipeline (BTC K-line + news sentiment + decision tree + InfluxDB/MongoDB sinks)
> - `plugins/bee-plugin-{binance,google-news,influxdb,mongodb,ta-lib}-mock/` — four independent `cdylib` mock plugins (one per Datasource; no business code in Bee core)

## Design Goals

| Goal | Detail |
| --- | --- |
| Zero runtime deps beyond `tokio` + `bytes` + `bincode` | Single binary, no JDK / ZK / external KV |
| Custom BRP binary protocol: 15-byte fixed header + bincode body | Tight wire format, no negotiation overhead |
| Hybrid: Data Plane P2P, Control Plane Raft | High throughput + strong consistency, each on its own channel |
| `RequestID` multiplexing & sliding-window backpressure | Natural async backpressure without explicit flow control |
| DAG-driven SQL / Lua DSL | Compilable, type-checked Pipelines |
| Auto-sharing of rate-limited Datasources (Producer Pipeline pattern) | N Pipelines use 1 external connection |
| First-class plugin system (Handler / Adapter dynamic libraries) | New Datasource = 2-hour plugin, not 2-week framework change |
| Automatic Work-Stealing + Migrating on node failure | Business 0-aware of transient failures |

## Architecture in One Sentence

Bee compiles user-authored SQL / Lua Pipelines into Phase DAGs; the Control Plane uses Raft to arbitrate which Job / Task runs on which Node; the Data Plane uses BRP to P2P-transmit Phase-to-Phase business flows between Nodes. Any node failure triggers Work-Stealing after 3× heartbeat; multiple Pipelines sharing a rate-limited Datasource share 1 Producer Pipeline rather than N connections.

> For the full design, see [docs/architecture.md](docs/architecture.md) and [docs/product-design.md](docs/product-design.md).

## Repository Layout

```
.
├── README.md              # This file
├── CONTEXT.md             # Domain glossary
└── docs/
    ├── product-design.md  # Product design document
    ├── architecture.md    # Technical architecture (OSS format)
    ├── internals.md       # Implementation details
    └── adr/               # Architecture decision records
        ├── 0001-data-plane-p2p-control-plane-raft.md
        ├── 0002-datasource-is-a-phase.md
        ├── 0003-producer-pipeline-pattern.md
        ├── 0004-bee-kv-cluster.md
        ├── 0005-plugin-ffi-rust-cdylib-mvp.md
        ├── 0006-sql-runtime-datafusion.md
        ├── 0007-simplified-raft-topology-mvp.md
        ├── 0008-optimizer-scheduler-adaptive.md
        ├── 0009-plugin-multiversion-hash-abi.md
        └── 0010-datasource-managed-entity.md
```

> Source directories (`Cargo.toml`, `crates/`, …) will be created in roadmap stage 0.1.

## Roadmap

> User-visible milestones. Detailed technical milestones in [docs/architecture.md §12.1](docs/architecture.md#121-roadmap) and the full backlog in [docs/stories.md](docs/stories.md).

- [ ] **0.1 – 0.2** Single-node works: `bee run pipeline.sql` shows the stream. Demo to seed users.
- [ ] **0.3 – 0.4** Small cluster: 3-node Failover demo. **First external paying user**.
- [ ] **0.5** Rate-limit sharing + cross-Pipeline: scenario A (quant) goes to production, **first quant strategy in production**.
- [ ] **0.6 – 0.7** Plugin system (Rust plugins) + scheduling optimizer. **3 external Adapters in the community**.
- [ ] **0.8** SQL performance tuning: ms-level micro-batch / UDF profiling / Hint syntax. **Quant scenario tunable**.
- [ ] **0.9 – 1.0** Production-ready: observability panel + Schema evolution. **Public 1.0 announcement**.
- [ ] **1.x** Enterprise features + Lua runtime + docs site + training.
- [ ] **2.x** Managed Cloud pilot + plugin marketplace (open C ABI + multi-language plugins).

## Contributing

The project is in early stages. Ways to contribute:

- **Design discussion**: open an Issue to challenge protocols / interfaces / tradeoffs.
- **Implementation**: PRs will open after the 0.1 roadmap stage.
- **Documentation**: PRs to fix this README or any file under `docs/` are welcome.

Until a public contributing guide is published, follow the principle: **open an Issue first, then submit a PR**.

## License

TBD (will be specified when the first crate is published; leaning Apache 2.0).
