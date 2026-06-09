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
| [docs/stories.md](docs/stories.md) | Generic Bee feature set: **32 implementation stories + S41 spike** (S0–S31 + performance showcase). Quant trading stories (S33–S40) live in [docs/best-practices/quant/stories.md](docs/best-practices/quant/stories.md). | Implementers |
| [docs/adr/](docs/adr/) | Irreversible architecture decision records (ADR) | Implementers |

## Quickstart (after implementation)

> Once the first AFK stories land, the canonical "5-minute end-to-end demo" lives at:
>
> - **`scripts/demo-perf.sh`** — the **5-minute performance showcase**: Fibonacci streaming + distributed prime sieve + multi-stream analytics, with a measured performance table across 1 / 3 / 5 Nodes. This is the demo a new evaluator can run to see "what does Bee actually do, fast?" — independent of any third-party service. See [examples/performance/README.md](examples/performance/README.md) and [docs/product-design.md §4.1](docs/product-design.md#41-performance-showcase)
> - The canonical 5-minute end-to-end demo will land as
>   `scripts/demo-perf-prod.sh` (S41 performance showcase, in flight).
> - The quant-trading reference implementation lives separately at [docs/best-practices/quant/](docs/best-practices/quant/); the production e2e demo is [`scripts/demo-quant-prod.sh`](scripts/demo-quant-prod.sh) (real Binance WS / NewsAPI / InfluxDB v2 / MongoDB; requires credentials in `scripts/.env`).

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
        ├── 0010-datasource-managed-entity.md
        └── 0011-stream-identity-and-backfill.md
```

> Examples live under `examples/` and `docs/best-practices/quant/examples/`:
> - `docs/best-practices/quant/examples/quant_btc_strategy.sql` + `quant_btc_strategy_backfill.sql` + `quant_btc_strategy_v2.sql` — the production quant-trading reference pipelines (S40)
> - `examples/performance/` — the Fibonacci + prime sieve + multi-stream analytics demos (S41)
>
> Example plugins under [plugins/quant/](plugins/quant/) are
> quant-trading reference implementations (binance / google-news /
> influxdb / mongodb / ta-lib). Their plugin STRUCTURE is
> production-grade; the data sources are placeholders. They live
> under [docs/best-practices/quant/](docs/best-practices/quant/) as
> real-world business examples.

> Source directories (`Cargo.toml`, `crates/`, …) will be created in roadmap stage 0.1.

## Roadmap

> User-visible milestones. Detailed technical milestones in [docs/architecture.md §12.1](docs/architecture.md#121-roadmap) and the full backlog in [docs/stories.md](docs/stories.md).

- [ ] **0.1 – 0.2** Single-node works: `bee run pipeline.sql` shows the stream. Demo to seed users.
- [ ] **0.3 – 0.4** Small cluster: 3-node Failover demo. **First external paying user**.
- [ ] **0.5** Rate-limit sharing + cross-Pipeline: scenario B (real-time multi-source monitoring) goes to production; quant reference implementation lands in [docs/best-practices/quant/](docs/best-practices/quant/).
- [ ] **0.6 – 0.7** Plugin system (Rust plugins) + scheduling optimizer. **3 external Adapters in the community** + performance showcase demo (S41).
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

## Performance Demos

The performance showcase is the new primary 5-minute demo of the
main repo. It runs 3 demo pipelines end-to-end and prints a measured
performance table:

- **Fibonacci**: 1M values via stateful `fib_step` UDF + KV-backed state
- **Prime sieve**: 10^8 integers via 20 sequential sieving Phases (correctness: count=12,779,448)
- **Multi-stream analytics**: 3 input streams + JOIN + GROUP BY

```bash
scripts/demo-perf.sh
```

See [`examples/performance/README.md`](examples/performance/README.md) for the math, the Bee design choices, and how to read the numbers.

## License

TBD (will be specified when the first crate is published; leaning Apache 2.0).

## Quant trading reference

The quant-trading implementation (S33 HITL milestone + S34–S40
production plugins + e2e deploy) is a large, real-world business
example. It lives in its own documentation section:

- [docs/best-practices/quant/](docs/best-practices/quant/) — stories,
  ADRs, examples, demo scripts, design specs.

The three production SQL pipelines (S40) are at:

- [`docs/best-practices/quant/examples/quant_btc_strategy.sql`](docs/best-practices/quant/examples/quant_btc_strategy.sql) — the canonical e2e pipeline (binance WS + Google News + MACD/EMA/RSI + price_direction + InfluxDB + MongoDB)
- [`docs/best-practices/quant/examples/quant_btc_strategy_backfill.sql`](docs/best-practices/quant/examples/quant_btc_strategy_backfill.sql) — backfill warmup variant (`from => '2024-06-01'`)
- [`docs/best-practices/quant/examples/quant_btc_strategy_v2.sql`](docs/best-practices/quant/examples/quant_btc_strategy_v2.sql) — a second strategy on the SAME binance Stream (demonstrates Producer sharing)

The end-to-end runner is at
[`scripts/demo-quant-prod.sh`](scripts/demo-quant-prod.sh). It
builds the 6 production plugins (S34–S39), registers 4 Datasources
(binance, google_news, influxdb, mongodb) with **connection-level**
config only, and deploys the 3 pipelines above. **Requires real
credentials in `scripts/.env`** (see
[`scripts/.env.example`](scripts/.env.example)) and local InfluxDB
v2 + MongoDB.

The 6 production plugin crates are at:

- [`plugins/quant/`](plugins/quant/) — S34 binance, S35 google-news, S36 influxdb, S37 mongodb, S38 ta-indicators
- [`plugins/bee-plugin-onnx-ml/`](plugins/bee-plugin-onnx-ml/) — S39 onnx-ml (FinBERT + tract)

All are part of the workspace members.
