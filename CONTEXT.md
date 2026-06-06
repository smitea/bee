# Bee

A distributed dataflow pipeline compute service. User-authored Pipelines are decomposed into Datasource, Pipeline, Phase, and Handler; Pipeline instances are scheduled and failover-managed by a Raft-backed Control Plane.

## Architecture

**数据面 (Data Plane)**:
The P2P traffic layer that carries Phase-to-Phase data, remote Handler invocations, and Input/Output flows over BRP. No coordinator sits in the data path.
_Avoid_: "去中心化网络" (misleading — Bee is hybrid, not pure P2P)

**控制面 (Control Plane)**:
The Raft-replicated authoritative state for membership, Pipeline/Phase ownership, orphan detection, and Work-Stealing arbitration. All mutations go through the Raft Leader.
_Avoid_: "中心服务器" (it is a replicated state machine, not a single server)

**Raft 集群 (Raft Cluster)**:
The replicated state machine that backs the Control Plane. Every ownership or membership change requires quorum commit.

## Topology

**节点 (Node)**:
A single Bee process / OS-level deployment unit. Each Node runs one Raft participant (in the simplest topology) and acts as a worker in the Data Plane.
_Avoid_: using 节点 for a vertex inside a Pipeline — that is a Phase (a 顶点 / vertex of the DAG).

## Data Model

**Pipeline**:
A user-authored, named DAG of Phases. Phases may emit to Phases in other Pipelines (cross-Pipeline edges) or fork to multiple successors. Static and immutable once compiled.
_Avoid_: using "Pipeline" for a running instance — that is a Pipeline Job.

**Phase**:
A 顶点 (vertex) in a Pipeline DAG. Has one or more typed input streams and one or more output streams; calls a Handler to transform them.
_Avoid_: using "Phase" for a scheduled instance — that is a Phase Assignment.

**Handler**:
A pure compute function invoked by a Phase. The runtime treats Handlers as stateless; any persistent per-Job state lives at the Job or Phase-Assignment level.

**Datasource**:
A Phase that wraps an external-system connection. The "Datasource" label is a role, not a type — at runtime a Datasource Phase is an ordinary Phase whose Adapter is an Input or Output plugin. Same lifecycle, same Work-Stealing, same Raft registration.
_Avoid_: treating Datasource as a first-class object with its own ID and lifecycle (use Adapter for version/config; the Phase carries the runtime identity).

**Adapter**:
The plugin contract for talking to an external system. Implemented as a dynamically loaded library; provides Input (subscribe/pull) and Output (emit/push) kinds. A Datasource Phase references exactly one Adapter plus a config payload.

**Cross-Pipeline Edge (跨 Pipeline 边)**:
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

**KV 集群 (KV Cluster)**:
A cluster-shared key-value store that runs as a second logical state machine on the same Raft cluster as the Control Plane. Stores Task State, Checkpoints, and Saved Offsets. Provides linearizable reads/writes and multi-key transactions. API: `get / put / cas / txn(ops)` over opaque bincode values; no range scan or index in MVP.
_Avoid_: thinking of it as a "time-series database" or "shared cache" — it is the durable state backbone of Bee, not an analytics store.

**Task State**:
Per-Task private state, owned by exactly one Task at a time, stored in the KV Cluster under a key derived from the TaskId. Default caps: 1 GB / Task, 7-day TTL after Job stops. The Handler maintains an in-memory hot cache and syncs to KV on a configurable cadence.
_Avoid_: storing business data (K-lines, news) directly in Task State — that goes through the Data Plane.

**Checkpoint**:
An atomic snapshot of `(Task State, Saved Offset)` for a Task, stored in the KV Cluster. The granularity of snapshotting and the recovery protocol are described in architecture.md §8.2.

## Plugin System

**Plugin**:
A dynamically loaded Rust crate (`.so` / `.dylib` / `.dll`) that implements one or more Adapters or Handlers. Compiled as `crate-type = ["cdylib"]`; exposed via `#[no_mangle] extern "C" fn bee_plugin_init(host: *mut BeeHost) -> *mut PluginHandle`. The Plugin Manager loads plugins from a configured directory and registers their Adapters / Handlers with the local Registry. MVP scope is Rust plugins only; C ABI for other languages is a 1.x concern (ADR-0005).
_Avoid_: plugins in other languages for MVP (the plugin author must match Bee's Rust toolchain version — a deliberate trade-off).

**Plugin Manifest**:
Metadata about a Plugin (logical name, feature version, **abi_version**, content hash, list of Adapters / Handlers it provides, configuration schema). The content hash and the abi_version are the binding truth; the feature version is human-readable. Stored in the Registry (local + network sync) for visibility and version compatibility checks. Used by the Compiler to validate Pipeline definitions against available Plugins.

**Plugin Identity (PluginId)**:
The content-hash-based unique identifier for a loaded Plugin: `PluginId = hex(sha256(plugin_binary_content))`. Two different builds of the same logical Plugin (even if they claim the same version string) have distinct PluginIds. The KV state key includes the hash (`state/task/{TaskId}/h{hash}/...`), and Pipelines bind to Plugins by PluginId, not by version string (ADR-0009).
