# Architecture Decision Records (ADRs)

This directory contains the architectural decisions made during the v2 design of Bee. Each ADR captures **what was decided, why, and the consequences** of the decision. ADRs are the durable record of "we picked this approach over that one" — they exist so future readers do not relitigate decisions and so a future Bee maintainer can understand *why* the system is the shape it is.

## Index

| # | Title | Status | Scope |
| --- | --- | --- | --- |
| [0001](./0001-data-plane-p2p-control-plane-raft.md) | Data Plane P2P + Control Plane Raft | Accepted | Architecture |
| [0002](./0002-datasource-is-a-phase.md) | Datasource is a Phase with an Adapter | Accepted | Data Model |
| [0003](./0003-producer-pipeline-pattern.md) | Producer Pipeline pattern for rate-limited Datasource sharing | Accepted | Sharing |
| [0004](./0004-bee-kv-cluster.md) | Bee KV Cluster for shared Task State | Accepted | State |
| [0005](./0005-plugin-ffi-rust-cdylib-mvp.md) | Plugin FFI boundary — Rust cdylib for MVP | Accepted | Extensibility |
| [0006](./0006-sql-runtime-datafusion.md) | SQL Runtime — DataFusion with extensions; Lua and other DSLs deferred to 1.x | Accepted | DSL |
| [0007](./0007-simplified-raft-topology-mvp.md) | Simplified all-in-one Raft topology for MVP | Accepted | Deployment |
| [0008](./0008-optimizer-scheduler-adaptive.md) | Optimizer and Scheduler responsibilities; runtime adaptive optimization with MLFQ default | Accepted | Runtime |
| [0009](./0009-plugin-multiversion-hash-abi.md) | Plugin multi-version coexistence; hash-based identity; strict ABI compatibility | Accepted | Plugins |
| [0010](./0010-datasource-managed-entity.md) | Datasource as a managed entity with `use` syntax and tenant namespace | Accepted | Management |
| [0011](./0011-stream-identity-and-backfill.md) | Stream identity scope & backfill-on-subscribe semantics | Accepted | Sharing |

## Conventions

- **Numbering**: sequential, never re-used. New ADR = previous max + 1.
- **Length**: typically 1-3 paragraphs of decision + 1 short "Consequences" section. This project's ADRs run slightly longer because the consequences matter for non-obvious downstream effects.
- **Don't modify accepted ADRs**. If a decision is reversed, write a new ADR that supersedes it (`Status: superseded by ADR-NNNN`).
- **When to write one**: all three of (1) hard to reverse, (2) surprising without context, (3) real trade-off. If any is missing, skip — the decision either does not need recording, or will be reversed cheaply, or was the obvious path.

## Decisions deferred to 1.x

| Source | Deferred item | When to revisit |
| --- | --- | --- |
| ADR-0005 | Full C ABI for non-Rust plugins | When first non-Rust plugin is requested |
| ADR-0006 | Lua runtime (via mlua) | When first Lua use case surfaces |
| ADR-0006 | Other DSLs (Python, JSON, YAML) | When first non-SQL DSL use case surfaces |
| ADR-0007 | Dedicated control plane (topology B) | When any of the three trigger conditions fire (Raft p99 > 10ms for 1 week / worker pool > 50 Nodes / explicit user request) |
| ADR-0007 | Split KV into its own Raft groups (TiKV-style) | When KV write throughput outgrows shared Raft group |
| ADR-0008 | Real preemption (OS cgroup controls) | When cooperative scheduling proves insufficient |
| ADR-0008 | Adaptive runtime DAG reordering | When static optimizer proves insufficient |
| ADR-0009 | Online state migration (`bee plugin migrate`) | When first "must migrate old state" scenario surfaces |
| ADR-0010 | Multi-tenant access enforcement (MVP carries `tenant: u16` struct field but does not check ACL) | 1.x when first multi-tenant deployment appears |
| ADR-0010 | External secret store (HashiCorp Vault / AWS Secrets Manager) integration | 1.x when compliance requirements mandate it |
| product-design | C ABI plugin marketplace | 2.x (per roadmap) |

## Known open questions (not yet ADRed)

The following items were noticed during v2 design but not formalised into ADRs. They are real design questions to be resolved before they become blocking.

- **Backpressure**: per-stream flow control beyond the BRP-level sliding window. Currently a per-Phase output buffer; what does end-to-end flow control look like across a DAG? *Estimated blocker: 0.5 (when cross-Pipeline edges are wired up).*
- **Schema evolution**: when a Pipeline Author changes the output type of a Phase, what happens to downstream subscribers? *Estimated blocker: 0.8 / 1.0 (production-readiness).*
- **Multi-tenancy / isolation**: how do multiple users / teams share a Bee cluster safely? Per-tenant quotas, namespacing, RBAC. *Estimated blocker: 1.x (Enterprise).*
- **Observability surface**: what metrics, traces, logs are exposed by default? *Estimated blocker: 0.8 (production-readiness).*
- **Hot-reload semantics**: when a Plugin is upgraded mid-Pipeline, what exactly happens to in-flight events? ADR-0009 covers state isolation; the event-level protocol is TBD. *Estimated blocker: 0.6 (Plugin system in).*
