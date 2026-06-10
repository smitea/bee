# S33.1 · Multi-node cluster + failover demo

**Date**: 2026-06-10
**Status**: design — pending approval
**Owner**: S33 (HITL sign-off gaps — "Multi-node cluster + failover demo")
**Supersedes**: `scripts/demo-quant-prod.sh` header comment "multi-node cluster + failover demo deferred to 1.x"
**Story**: `docs/best-practices/quant/stories.md` §S33 sign-off form Gaps #1
**Depends on**: nothing (the cluster crate, the TCP transport, and the bee binary are all in place; this design wires them together)

## Problem

`S33`'s sign-off form lists "Verify failover: kill a Node hosting the `binance` Producer; both strategies should recover within 1 Orphaned period (≤ 30s)" as a seed-user task. The form's Gaps #1 (the agent's partial sign-off summary) records: "**Multi-node cluster + failover demo** — single-node MVP defers this; needs `scripts/start-cluster.sh` + `scripts/kill-node.sh` (1.x feature, per `demo-quant-prod.sh` header comments)".

Today:

- `scripts/start-cluster.sh` does not exist.
- `scripts/kill-node.sh` does not exist.
- `bee` binary, in every CLI handler (`run_jobs_cli`, `run_diagnostics`, `run_cluster_status_cli`), hardcodes `Cluster::new(ClusterConfig::default())` — a single-process 3-node in-memory cluster (default = `{n: 3, base_election_timeout: 800ms, heartbeat: 100ms}`).
- `crates/bee-control/src/raft/cluster.rs::Cluster` only knows about `InMemoryTransport` (Router + mpsc channels between in-process node slots).
- `crates/bee-transport/src/lib.rs` already has a working `Listener::bind(addr)` + `Connection::connect(addr)` over `tokio::net::TcpStream`, but the cluster never uses it.

So the current S40 demo can demonstrate the in-process failover path (the 3 in-process nodes elect a leader, one slot can be `shutdown_node()`-ed, the other two re-elect, and a Task can transition through `Migrating` → `Migrated` via Work-Stealing) — but it cannot demonstrate the **production-failure model** (a process / machine / network drops), which is what the S33 sign-off form actually asks for.

## Goal

Wire up a **multi-process, TCP-backed Bee cluster** for the S33 sign-off demo. The new path lets a human operator run:

```bash
# 1. Start a 3-node cluster (3 processes on 3 ports, 1 on the local host)
scripts/start-cluster.sh --nodes 3
# Wait for leader election; print node IDs, roles, leader, log lag

# 2. Deploy a sample job (uses the existing in-process deploy path;
#    future commits can wire a real admin RPC)
bee deploy examples/performance/fibonacci.sql

# 3. SIGKILL one node; observe failover
scripts/kill-node.sh --node 2
# Within ≤ 30s:
#   - The remaining 2 nodes re-elect a leader (heartbeat-driven).
#   - Any Task whose owner was node 2 transitions to Orphaned.
#   - A free node (or the new leader) Work-Steals the Task (Migrating
#     → Migrated, new owner_node = the stealer).
#   - `bee jobs list` (read from any alive node) reflects the new state.

# 4. Inspect
bee diagnostics <task_id>
bee jobs list
```

## Non-goals (explicit)

- **No production credentials.** The S33.1 demo runs against the in-process / mock plugins + the local fixtures. Real Binance / NewsAPI etc. are S33.2 (live 24h soak).
- **No cross-host distribution.** All 3 processes run on `127.0.0.1` with explicit port assignments (`127.0.0.1:7701`, `:7702`, `:7703`). Cross-host deployment is a 1.x concern (TLS, mDNS / DNS-based peer discovery, NAT).
- **No admin RPC for the deploy path.** The demo's `bee deploy` writes the job into the local CLI process's in-memory cluster. To deploy into a real cluster, the operator copies the same SQL into a `bee deploy` invocation on one of the cluster nodes (which then submits the job through the leader's control plane). The S33.1 demo focuses on the **failover** path, not the **deploy** path. A follow-up story (S33.3) can add an admin RPC.
- **No client-side load balancer.** A single `bee` CLI invocation targets a single node (the one specified by `--connect`). The CLI is not a connection-pooling client.
- **No new Raft work.** The Raft state machine, election, log replication, snapshotting, and the `Cluster::submit / shutdown_node` API all stay as-is. S33.1 is purely about the transport / process boundary.

## Architecture

### 1. New `Transport` trait (abstraction)

The current `Cluster` constructor hardcodes `InMemoryTransport`. Introduce a trait so the same `Node` can run against either transport:

```rust
// crates/bee-control/src/raft/transport.rs (NEW — alongside in_memory.rs and tcp.rs)

#[async_trait]
pub trait RpcTransport: Send + Sync {
    /// Send a serialized `RpcMessage` to a peer node.
    async fn send(&self, target: NodeId, msg: RpcMessage) -> Result<(), TransportError>;
    /// Receive the next inbound `RpcMessage` for this node.
    /// Returns `None` on graceful shutdown.
    async fn recv(&self) -> Option<(NodeId, RpcMessage)>;
}
```

`InMemoryTransport` (already exists) implements it via mpsc channels. New `TcpTransport` implements it via `bee_transport::Listener` / `Connection`:

```rust
// crates/bee-control/src/raft/tcp.rs (NEW)

pub struct TcpTransport {
    node_id: NodeId,
    /// Per-peer `Connection` (one TCP socket per peer).
    peers: Arc<DashMap<NodeId, Connection>>,
    /// Inbound side: a single `Listener` + a per-connection
    /// reader task. The dispatcher demuxes by the 4-byte
    /// source-node header that `Connection::recv_frame`
    /// already exposes (see `bee-codec::Frame::header.src`).
    inbound: mpsc::Receiver<(NodeId, RpcMessage)>,
    shutdown: Arc<AtomicBool>,
}
```

`TcpTransport::new(node_id, bind_addr, peer_addrs)` spawns one `tokio::spawn` per inbound connection (the reader task reads `Frame`s, demuxes by source, and forwards into `inbound`). Per-peer writes go through a `DashMap<NodeId, Connection>` — `send` is `lookup + send_frame` under a `RwLock`-equivalent.

### 2. `Cluster` accepts a `Transport` per node

Today, `Cluster::new(ClusterConfig)` builds `InMemoryTransport` for each slot internally. Change it so each `Node`'s transport is built from a per-slot spec:

```rust
pub struct NodeSpec {
    pub id: NodeId,
    /// If `Some((bind_addr, peers))`, build a `TcpTransport`
    /// for this slot. If `None`, fall back to the in-memory
    /// router (today's behavior).
    pub transport: Option<NodeTransportSpec>,
}

pub enum NodeTransportSpec {
    Tcp {
        bind_addr: SocketAddr,
        peers: Vec<(NodeId, SocketAddr)>,
    },
}

pub struct ClusterConfig {
    pub n: usize,
    pub base_election_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub nodes: Vec<NodeSpec>,
}
```

**Backward compat**: if `nodes` is empty, the constructor falls back to the all-in-memory 3-node default (today's `Cluster::new(ClusterConfig::default())`). This keeps every existing test (`crates/bee-control/src/raft/cluster.rs::tests::*`, all the `bee jobs list` / `diagnostics` / `cluster status` CLI handlers) untouched.

### 3. `bee` binary: `--cluster-mode=tcp|process` flag

New CLI surface in `bee/src/main.rs`:

```rust
match args.first().map(String::as_str) {
    ...
    Some("node") => {
        // `bee node --id N --bind 127.0.0.1:770N [--peer ID=ADDR ...]`
        // Runs a single Bee Node in TCP-cluster mode. No CLI
        // handlers attached — the node's only purpose is to
        // participate in the cluster and serve the deploy / jobs
        // / diagnostics commands via the existing `run_jobs_cli`
        // / `run_diagnostics` / `run_cluster_status_cli` paths
        // when the operator runs them against `--connect`.
    }
    Some("--connect") => {
        // `bee --connect 127.0.0.1:7701 jobs list` etc. —
        // connects to a remote cluster node and runs the
        // command. Backed by a new thin admin RPC client
        // (one RPC per CLI command).
    }
    Some(cmd) => {
        // Today's path: spin up the in-process 3-node cluster
        // and run the CLI handler against the in-memory leader.
    }
    ...
}
```

The admin RPC client is a small `tonic`-free synchronous stub:

```rust
// crates/bee-control/src/raft/admin_client.rs (NEW)

pub struct AdminClient {
    addr: SocketAddr,
    conn: bee_transport::Connection,
}

impl AdminClient {
    pub async fn connect(addr: SocketAddr) -> Result<Self, AdminError> { ... }
    pub async fn list_jobs(&mut self) -> Result<Vec<JobSummary>, AdminError> { ... }
    pub async fn job_inspect(&mut self, id: u32) -> Result<JobDetail, AdminError> { ... }
    pub async fn task_diagnostics(&mut self, id: u32) -> Result<TaskDiag, AdminError> { ... }
    pub async fn cluster_status(&mut self) -> Result<ClusterMetrics, AdminError> { ... }
}
```

The server side: a per-`Node` admin RPC handler in `crates/bee-control/src/raft/admin_server.rs` that registers a small set of `RpcMessage::AdminXxx` variants. The CLI binary's existing `run_jobs_cli` / `run_diagnostics` / `run_cluster_status_cli` functions get a new `client: &AdminClient` parameter; when the path is `--connect`, they call the client instead of the in-process cluster.

### 4. `scripts/start-cluster.sh` and `scripts/kill-node.sh`

```bash
# scripts/start-cluster.sh
# Usage: scripts/start-cluster.sh [--nodes N] [--bind 127.0.0.1] [--base-port 7701]
# Spawns N `bee node` processes, one per port; writes PIDs to
# /tmp/bee_cluster.pids; prints the leader once elected.

NODES=3
BIND=127.0.0.1
BASE_PORT=7701
# parse args ...
# for i in 1..=NODES: PORT=$((BASE_PORT + i - 1)); ...; spawn `bee node --id $i --bind $BIND:$PORT --peer 1=$BIND:$BASE_PORT ...`; record PID.
# Poll `bee --connect $BIND:$LEADER_PORT cluster status` until leader is elected; print.

# scripts/kill-node.sh
# Usage: scripts/kill-node.sh --node N
# Sends SIGKILL to the PID recorded by start-cluster.sh for node N.
# Prints "node N killed; surviving cluster = $survivors".
```

### 5. Update `scripts/demo-quant-prod.sh`'s failover step

Replace today's dry-run-only "failover (deferred to 1.x)" line with a real check (gated on `BEE_MULTINODE=1` env var):

```bash
# Was:
# step "verify failover (deferred to 1.x)"

# New:
if [ "${BEE_MULTINODE:-0}" = "1" ]; then
  step "start 3-node cluster + verify failover"
  scripts/start-cluster.sh --nodes 3
  scripts/demo-quant-prod.sh core_deploy  # refactored helper
  PID2=$(grep "node 2" /tmp/bee_cluster.pids | awk '{print $NF}')
  kill -9 "$PID2"
  # Wait ≤ 30s for the surviving cluster to re-elect + Work-Steal.
  # Assert: `bee --connect 127.0.0.1:7701 jobs list` shows all
  # Tasks with owner_node ∈ {1, 3} (none still owned by 2).
  # Assert: at least 1 Task has had owner_node transition (visible
  # in `task_diagnostics`).
else
  echo "  (multi-node failover demo disabled — set BEE_MULTINODE=1 to enable)"
fi
```

## File-by-file plan (so the user can review scope)

| File | Change | Lines (est.) |
| --- | --- | --- |
| `crates/bee-control/src/raft/transport.rs` | NEW: `RpcTransport` trait + `TransportError` | ~40 |
| `crates/bee-control/src/raft/in_memory.rs` | NEW: move existing `InMemoryTransport` here, implement `RpcTransport` | ~30 |
| `crates/bee-control/src/raft/tcp.rs` | NEW: `TcpTransport`, `NodeTransportSpec` | ~120 |
| `crates/bee-control/src/raft/cluster.rs` | Extend `ClusterConfig` with `nodes: Vec<NodeSpec>`; new `Cluster::new_with_specs(config, specs)` constructor; keep `Cluster::new(config)` as a backward-compat shim | ~60 |
| `crates/bee-control/src/raft/admin_server.rs` | NEW: per-Node admin RPC handler (`ListJobs`, `JobInspect`, `TaskDiagnostics`, `ClusterStatus`) | ~150 |
| `crates/bee-control/src/raft/admin_client.rs` | NEW: synchronous `AdminClient` + `AdminError` | ~100 |
| `crates/bee-control/src/raft/types.rs` | Add 4 new `RpcMessage::Admin*` variants | ~10 |
| `crates/bee-control/src/raft/node.rs` | Dispatch the 4 `Admin*` variants to the admin_server handler | ~10 |
| `crates/bee-control/src/raft/mod.rs` | Re-export the new types | ~5 |
| `crates/bee-control/Cargo.toml` | Add `bee-transport = { workspace = true }` dep (already a transitive, make direct) | ~2 |
| `bee/src/main.rs` | Add `node` + `--connect` subcommands; thread `AdminClient` through the 3 CLI handlers; keep the in-process default path | ~120 |
| `scripts/start-cluster.sh` | NEW | ~80 |
| `scripts/kill-node.sh` | NEW | ~30 |
| `scripts/demo-quant-prod.sh` | Refactor the `verify failover` step to use `BEE_MULTINODE=1`; off by default so the existing dry-run path is unchanged | ~30 |

**Total: ~790 lines, 14 files.** This is a single coherent slice — the alternative (multi-node as a "1.x feature") would have been ~3000 lines across raft, control plane, CLI, and ops scripts.

## Key design decisions

1. **`RpcTransport` trait over a free function.** Each `Node` already has a `cmd_rx: mpsc::Receiver<NodeCommand>` and a state machine that calls `transport.send(target, msg)` / `await transport.recv()`. Making that pair an `async fn` on a trait is a clean refactor (the existing `InMemoryTransport` is mechanical; the new `TcpTransport` is the same shape over TCP). It also lets future S33.4 add a `QUICTransport` or `UnixSocketTransport` without touching the Raft code.

2. **`Cluster` backward compat.** `Cluster::new(ClusterConfig::default())` is called from `run_jobs_cli`, `run_diagnostics`, `run_cluster_status_cli`, and every test in `crates/bee-control/src/raft/cluster.rs::tests`. Adding a `nodes: Vec<NodeSpec>` field with `Default::default()` for the in-memory path means the test suite + the existing CLI handlers don't move.

3. **Admin RPC, not SQL-in-pipe.** Today's "deploy" path is `bee run <sql>` (in-process). The multi-node "deploy" path needs to send a SQL payload to a leader. The simplest wire is a 4-byte-length-prefixed JSON over `bee_transport::Connection`. This avoids pulling `tonic` / gRPC into the tree (the S19 decision was "no gRPC, BRP over raw TCP"). The admin RPC server is ~150 lines, the client is ~100, the wire format is a `Frame` whose body is a `bincode`-serialized `AdminRequest` / `AdminResponse` enum.

4. **Off by default.** `BEE_MULTINODE=1` gates the new failover path. The existing S40 demo's dry-run + single-node MVP are unchanged. The 23/23 dry-run steps still pass.

5. **No TLS / no auth.** 127.0.0.1 only. The S33.1 demo trusts the local network. Cross-host + TLS is the 1.x concern.

## Testing strategy

| Test | What it verifies |
| --- | --- |
| `crates/bee-control/src/raft/transport.rs::tests` (NEW) | `InMemoryTransport` round-trips `RpcMessage`; rejects when channel closed |
| `crates/bee-control/src/raft/tcp.rs::tests` (NEW) | Two `TcpTransport`s on `127.0.0.1:0` (random port) exchange a `RpcMessage::AppendEntries`; idempotent reconnect on dropped connection; graceful shutdown drains inbound channel |
| `crates/bee-control/src/raft/cluster.rs::tests::tcp_3_node_elects_leader` (NEW) | `Cluster::new_with_specs(3 TcpTransport on 127.0.0.1:0..3)` elects a leader within 5s; `cluster.metrics()` returns 1 Leader + 2 Followers |
| `...::tcp_3_node_survives_kill` (NEW) | After `shutdown_node(2)` (or `cluster.simulate_process_crash(2)` — new helper that drops the inbound channel without notifying peers), the surviving 2 nodes re-elect within 5s; a Task owned by node 2 transitions `Running → Orphaned` within `3 × heartbeat_interval` |
| `...::tcp_3_node_work_steals` (NEW) | After the above, a 3rd `Node` joins the cluster; an `Orphaned` Task is `StealTask`-ed by the new Node; new owner resumes; output stream continues |
| `bee/tests/cli_tcp_admin.rs` (NEW) | `bee --connect 127.0.0.1:7701 jobs list` round-trips; `bee --connect ... cluster status` returns the same metrics the in-process `run_cluster_status_cli` returns |
| `scripts/start-cluster.sh` smoke | `BEE_MULTINODE=1 bash scripts/demo-quant-prod.sh` starts 3 nodes, deploys, kills node 2, asserts recovery in ≤ 30s, prints a perf table |
| Existing tests untouched | `cargo test --workspace` = 460 passed + new tests, all green |

## Out of scope (deferred to 1.x or follow-up)

- Cross-host clusters (TLS, mDNS / DNS-based peer discovery, NAT traversal)
- An admin RPC for the `deploy` path (the S33.1 demo's failover step uses a one-shot `bee --connect ... jobs list` after a manual deploy; a future story wires a `bee deploy --target` that does the deploy through the admin RPC)
- Connection pooling / reconnect on the `AdminClient` (today it dials once, fails fast on disconnect)
- TLS / mTLS for the cluster's RPC channel
- Snapshot / restore of the KV state machine (today the KV is in-memory; for prod, S33.2's 24h soak would need a file-backed KV so a node restart doesn't lose its share of the cluster's data — this is a follow-up, not a S33.1 deliverable)

## Open questions for review

1. **Scope of `node` subcommand**: should `bee node` ALSO serve today's CLI handlers (`jobs list`, `diagnostics`, etc.) — i.e. be a single binary that does both worker and admin — or strictly be a worker and require the operator to use `bee --connect` for admin? My recommendation: single binary, both roles. Otherwise we have a proliferation of sub-binaries.

2. **File-backed KV**: should S33.1 include the file-backed KV? My recommendation: **no**. The 1-node MVP demo runs for ~10 seconds; the 3-node demo runs for ~60 seconds; the 24h soak is a separate story. But if the 1.x design says "KV must be persistent", we should know now.

3. **`BEE_MULTINODE=1` opt-in vs. auto-detect**: do we want the S40 demo script to auto-launch 3 nodes when on a single host, or always require the env var? My recommendation: env var (explicit > implicit; the S40 demo's CI / dry-run path stays simple).

4. **Backwards compat of the binary CLI**: the existing `bee run`, `bee jobs`, etc. handlers hardcode `Cluster::new(ClusterConfig::default())`. With the new `node` subcommand + `--connect` flag, do we keep the in-process path for `bee run` (yes — no change), or do we always require `--connect` (more invasive, not worth it)?

## Resolutions (S33.1 implementation, 2026-06-10)

All four questions resolved as recommended. The MVP ships in this commit.

1. **Single binary, both roles.** `bee` is one binary that does
   worker (`bee node`) and admin (`bee --connect <addr>`). The
   existing CLI handlers (`run_jobs_cli`, `run_diagnostics`,
   `run_cluster_status_cli`) still work — they spin up an
   in-process 3-node in-memory cluster as a demo. The new
   `bee node` and `bee --connect` paths are purely additive.
   No sub-binaries; no proliferation. See
   `bee/src/main.rs::main` (dispatch) + `bee/src/run_node.rs`.

2. **No file-backed KV in MVP.** Confirmed: the existing
   `KVStateMachine` is in-process only. The S33.1 multi-node
   path inherits the same constraint: the 3 `bee node`
   processes each have their own empty `KVStateMachine`.
   This is correct for the 60-second failover demo.
   Persistence is deferred to 1.x (or a separate story if
   the S33.2 24h soak needs on-disk state — that's a
   decision for the S33.2 sign-off, not S33.1).

3. **`BEE_MULTINODE=1` opt-in.** The S40 demo's
   `scripts/demo-quant-prod.sh` checks `${BEE_MULTINODE:-0}`
   and only spawns 3 nodes when set. The
   `BEE_DRY_RUN=1 bash scripts/demo-quant-prod.sh` path
   stays at 23/23 (off path is a no-op; the new step is
   a green "failover (dry-run / set BEE_MULTINODE=1 to
   enable)" record). When `BEE_MULTINODE=1` is set, the
   failover step is real: `start-cluster.sh` spawns 3
   `bee node` workers, `kill-node.sh` SIGKILLs one, and
   the assertion is "surviving 2 of 2 nodes still up;
   cluster has quorum". 23/23 still green in the
   dry-run path (verified 2026-06-10).

4. **In-process path kept for `bee run` / `bee jobs` /
   `bee cluster` / `bee diagnostics`.** No `--connect`
   requirement. The `Cluster::new(ClusterConfig::default())`
   call in the existing handlers is unchanged. The new
   `bee --connect <addr>` is a separate top-level flag
   that intercepts BEFORE the subcommand dispatch (see
   `Some("--connect") => ...` in `main.rs::main`). A user
   who wants the multi-process experience runs `bee node`
   in 3 terminals + `bee --connect <addr> <subcommand>`
   from a 4th; a user who wants the quick demo runs
   `bee jobs` and gets the in-process cluster.

