# Bee

A distributed dataflow pipeline compute service. User-authored Pipelines are decomposed into Datasource, Pipeline, Phase, and Handler; Pipeline instances are scheduled and failover-managed by a Raft-backed Control Plane.

## Architecture

**Data Plane**:
The P2P traffic layer that carries Phase-to-Phase data, remote Handler invocations, and Input/Output flows over BRP. No coordinator sits in the data path.
_Avoid_: "decentralized network" (misleading — Bee is hybrid, not pure P2P)

**Control Plane**:
The Raft-replicated authoritative state for membership, Pipeline/Phase ownership, orphan detection, and Work-Stealing arbitration. All mutations go through the Raft Leader.
_Avoid_: "central server" (it is replicated, not standalone)

**Raft Cluster**:
The replicated state machine that backs the Control Plane. Every ownership or membership change requires quorum commit.

## Topology

**Node**:
A single Bee process / OS-level deployment unit. Each Node runs one Raft participant (in the simplest topology) and acts as a worker in the Data Plane.
_Avoid_: using "Node" for a vertex inside a Pipeline — that is a Phase (a vertex of the DAG).

## Data Model

**Pipeline**:
A user-authored, named DAG of Phases. Phases may emit to Phases in other Pipelines (cross-Pipeline edges) or fork to multiple successors. Static and immutable once compiled.
_Avoid_: using "Pipeline" for a running instance — that is a Pipeline Job.

**Phase**:
A vertex in a Pipeline DAG. Has one or more typed input streams and one or more output streams; calls a Handler to transform them.
_Avoid_: using "Phase" for a scheduled instance — that is a Phase Assignment.

**Handler**:
A pure compute function invoked by a Phase. The runtime treats Handlers as stateless; any persistent per-Job state lives at the Job or Phase-Assignment level.

**Datasource (managed Provider)**:
A named, registered **Provider** in Bee that bundles a connection to an external system — its name, the Adapter it wraps, its **connection-level** configuration (credentials, base URL, rate-limit settings), its lifecycle status, and the tenant it belongs to. A Datasource does NOT carry per-call arguments (symbol, interval, query string); those go in the SQL at the call site. Pipelines reference Datasources via `use <name>;` and call Adapter methods with per-call args (e.g., `binance.subscribe('BTC/USDT', '5min')`).
_Avoid_: confusing Datasource (Provider = connection) with Stream (the result of a specific call). Two different calls on the same Datasource produce two different Streams and may have two different Producers (per ADR-0003 refined by ADR-0010).

**Adapter**:
The plugin contract for talking to an external system. Implemented as a dynamically loaded library; provides Input (subscribe/pull) and Output (emit/push) kinds. A Datasource Phase references exactly one Adapter plus a config payload. The Adapter supplies the method names (e.g., `subscribe`, `emit`) that Pipelines call after `use`-ing the Datasource.

**Cross-Pipeline Edge**:
An edge in the DAG whose source Phase and target Phase belong to different Pipelines. The runtime resolves the edge at deploy time: if both Phases are in the same Job, the edge is in-process; if they are in different Jobs, the edge is a BRP data-channel subscription.
_Avoid_: a separate "Inter-Pipeline Bus" type (the same BRP data channel carries every edge).

## Instances

**Pipeline Job**:
A running instance of a Pipeline, identified by `JobId`. Has its own lifecycle independent of the Pipeline definition.

**Phase Assignment (Task)**:
A running instance of a Phase, scheduled to a specific Node, identified by `TaskId`. The unit of failover and Work-Stealing.

**Producer Pipeline**:
A Pipeline Job that exists primarily to publish a stream that other Jobs subscribe to. The canonical case is a Datasource-as-Pipeline: a single-Phase Pipeline running one or more Input Adapters, with no other upstream. The Producer owns the rate-limited network connection; subscribers reuse its stream.
_Avoid_: introducing a separate "Shared Source Service" object — a Producer is just a Pipeline Job whose Phases happen to be Datasource Phases.

## Lifecycle States

**Orphaned**:
A Phase Assignment whose owner Node has missed heartbeats for `3 × heartbeat_interval` (default heartbeat_interval = 10s, so orphan at 30s). Eligible for Work-Stealing takeover.

**Migrating**:
A Phase Assignment being handed off from one Node to another (during Work-Stealing or planned rebalancing). The target Node reads the latest **Checkpoint** from the **KV Cluster**, restores Task State, then reconnects upstream to resume from the Saved Offset. The source Node, if recovered, drains its in-flight emissions and stops.

## State & Storage

**KV Cluster**:
A cluster-shared key-value store that runs as a second logical state machine on the same Raft cluster as the Control Plane. Stores Task State, Checkpoints, and Saved Offsets. Provides linearizable reads/writes and multi-key transactions. API: `get / put / cas / txn(ops)` over opaque bincode values; no range scan or index in MVP.
_Avoid_: thinking of it as a "time-series database" or "shared cache" — it is the durable state backbone of Bee, not an analytics store.

**Task State**:
Per-Task private state, owned by exactly one Task at a time, stored in the KV Cluster under a key derived from the TaskId. Default caps: 1 GB / Task, 7-day TTL after Job stops. The Handler maintains an in-memory hot cache and syncs to KV on a configurable cadence.
_Avoid_: storing business data (K-lines, news) directly in Task State — that goes through the Data Plane.

**Checkpoint**:
An atomic snapshot of `(Task State, Saved Offset)` for a Task, stored in the KV Cluster. The granularity of snapshotting and the recovery protocol are described in architecture.md §7.2.

## Plugin System

**Plugin**:
A dynamically loaded Rust crate (`.so` / `.dylib` / `.dll`) that implements one or more Adapters or Handlers. Compiled as `crate-type = ["cdylib"]`; exposed via `#[no_mangle] extern "C" fn bee_plugin_init(host: *mut BeeHost) -> *mut PluginHandle`. The Plugin Manager loads plugins from a configured directory and registers their Adapters / Handlers with the local Registry. MVP scope is Rust plugins only; C ABI for other languages is a 1.x concern (ADR-0005).
_Avoid_: plugins in other languages for MVP (the plugin author must match Bee's Rust toolchain version — a deliberate trade-off).

**Plugin Manifest**:
Metadata about a Plugin (logical name, feature version, **abi_version**, content hash, list of Adapters / Handlers it provides, configuration schema). The content hash and the abi_version are the binding truth; the feature version is human-readable. Stored in the Registry (local + network sync) for visibility and version compatibility checks. Used by the Compiler to validate Pipeline definitions against available Plugins.

**Plugin Identity (PluginId)**:
The content-hash-based unique identifier for a loaded Plugin: `PluginId = hex(sha256(plugin_binary_content))`. Two different builds of the same logical Plugin (even if they claim the same version string) have distinct PluginIds. The KV state key includes the hash (`state/task/{TaskId}/h{hash}/...`), and Pipelines bind to Plugins by PluginId, not by version string (ADR-0009).

**Tenant Namespace**:
A `uint16` identifier (0 to 65535) that scopes ownership of Datasources and Jobs. Tenant `0` is the global / public namespace; values 1-65535 are tenant IDs. A Job's tenant is set by the submission context (API key, CLI auth, etc.). A Job can `use` a Datasource only if `ds.tenant == job.tenant` or `ds.tenant == 0`. MVP carries the field but does not enforce the access rule; 1.x turns enforcement on (ADR-0010).

## Current state (snapshot — 2026-07-28)

- **Stories**: 140/156 acceptance criteria ticked (89.7%). Remaining 16 are explicitly deferred per their stories' specs (perf benchmarks, deferred bench/perf tests, pluggable follow-ups).
- **Tests**: 477 pass / 0 fail / 5 ignored (the 5 ignored are stubs awaiting `S-1c test-utils` future expansion).
- **GUI**: `app/` (Tauri 2.x + React + Vite + TypeScript + Tailwind) — replaces the iced `crates/bee-gui/`. All 4 tabs functional:
  - **Dashboard** (S-1a) — 3 stat cards + Nodes table + Recent Jobs table + auto-refresh 5s
  - **Data Sources** (S-2) — Datasource CRUD (create / list / inspect / pause / resume / delete) via in-process DatasourceRegistry (localStorage); production wires through AdminServer RPC
  - **Pipelines** (S-3/4) — Job list + Inspect panel via AdminServer `list_jobs` / `job_inspect`
  - **Settings** (S-5) — Theme picker + log level picker + diagnostic export + spec link
- **Backend** (unchanged by the Tauri switch):
  - S-07 Raft WAL (`crates/bee-control/src/raft/wal.rs`, magic `BEERAWL1`) — log + term persistence across restart
  - S-07-x Raft snapshot (`crates/bee-control/src/raft/snapshot.rs`, magic `BEERSNP1`) — WAL bounded via snapshot+truncate
  - S-1c test-utils (`crates/bee-control/src/test_utils.rs`) — public 3-node Cluster + AdminServer harness
- **Tauri backend** (`app/src-tauri/`):
  - `src/connection.rs` — port of `crates/bee-gui/src/connection.rs` (AdminClient lifecycle + tokio bridge)
  - `src/commands.rs` — `#[tauri::command]` per RPC: `ping`, `cluster_status`, `list_jobs`, `job_inspect`, `connection_state`
  - `src/lib.rs` — Tauri builder + setup; bootstraps AdminClient on `BEE_ADMIN_ADDR` env var (default `127.0.0.1:9999`)
- **CI** (`.github/workflows/rust.yml`):
  - `rust-build` job on ubuntu-latest (cargo build --workspace --release, pinned to rustc 1.89.0 via rust-toolchain.toml)
  - `rust-test` job on ubuntu-latest (cargo test --workspace --release)
  - `tauri-frontend-build` job on ubuntu-latest (Node.js 20 + npm ci + tsc --noEmit + vite build)
- **Toolchain**: rustc 1.89.0 (kstring 2.0.2 pin via Cargo.lock since kstring 2.0.3+ requires rustc 1.96+); Node.js 20 for the Tauri frontend

## Recently completed (2026-07-28 session)

| Commit | Spec area | Description |
|---|---|---|
| `feat/all-stories-cleanup` | S00-S49 | 84 acceptance criteria flipped from `[ ]` to `[x]` |
| `feat/s1a-gui-foundation` | S-1a | 13 commits — crates/bee-gui workspace member (iced), 29 Lucide icons, Dashboard, settings, error, log_panel |
| `feat/s1a-gui-launch` | S-1a | `iced::Application::run` wired; macOS M2 GUI verified |
| `feat/s1b-theme-switch` | S-1b | Light/Dark theme toggle in AppBar |
| `feat/s1c-test-utils` | S-1c | Public TestCluster harness; 4 ignored bee-gui tests enabled |
| `feat/s2-datasource-ui` | S-2 | Data Management page replaces placeholder |
| `feat/s07x-snapshot` | S-07-x | Raft snapshot file + WAL truncation |
| `feat/s5-settings-ui` | S-5 | Settings page replaces placeholder |
| `feat/s3-pipelines-ui` | S-3/4 | Pipelines page replaces placeholder |
| `feat/s1c-tooltip` | S-1c | Self-drawn tooltip widget on AppBar |
| `feat/ui-polish` | ui redo | fixes loading bug + English tabs + UI redesign |
| `feat/s1c-pipeline` | deprecation | merges s1a-gui-launch wiring |
| `feat/ci-setup` | infra | 3-job CI workflow + rust-toolchain.toml pin |
| `feat/docs-refresh` | docs | CONTEXT.md current-state snapshot |
| `feat/taui-frontend` | S-Tauri A+B | Tauri 2.x + React + Vite + Tailwind scaffold + 4 frontend pages |
| `feat/taui-cleanup` | S-Tauri C-E | removed iced, updated CI for Node.js + Tauri |

## See also

- Story backlog: [`docs/stories.md`](docs/stories.md) — 38 stories with acceptance criteria
- Tauri GUI design contract: [`docs/superpowers/specs/2026-07-28-s-tauri-gui-design.md`](docs/superpowers/specs/2026-07-28-s-tauri-gui-design.md) (status: **Implemented**)
- Original S-1a spec (iced, archived): [`docs/superpowers/specs/2026-07-27-s1a-gui-foundation-design.md`](docs/superpowers/specs/2026-07-27-s1a-gui-foundation-design.md) (status: **Superseded by S-Tauri**)
- ADRs: [`docs/adr/`](docs/adr/) (10 numbered ADRs covering wire format, control plane, KV, datasource model, plugin FFI, multi-version, SQL runtime, adaptive scheduling, plugin identity hash, datasource managed entity)
