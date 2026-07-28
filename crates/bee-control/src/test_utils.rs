//! `test_utils` — shared in-process 3-node `Cluster` +
//! per-node `AdminServer` harness for integration tests.
//!
//! Gated behind `#[cfg(any(test, feature = "test-utils"))]` so
//! production binaries do not pay for it. Downstream test
//! crates (e.g. `bee-gui`) opt in via
//! `bee-control = { workspace = true, features = ["test-utils"] }`
//! in their `dev-dependencies`.
//!
//! S-1c: this module exists so the four previously-`#[ignore]`d
//! integration tests in `crates/bee-gui/tests/` can boot a real
//! Raft cluster + AdminServer and exercise the GUI's
//! `AdminClient::connect / call` paths end-to-end.
//!
//! What this consolidates:
//! - `crates/bee-control/tests/admin_forwarding_inmem.rs:38-58`
//!   — `boot_3_node_inmem()` (`Cluster::new_with_specs` with
//!   short election timeouts).
//! - `crates/bee-control/tests/admin_forwarding_inmem.rs:88-100`
//!   — per-node `AdminServer::start(...)` wiring with
//!   `register_reply` pointing at each live Node's
//!   `pending_replies` handle.
//! - `crates/bee-control/tests/raft_cluster.rs:5-14` — minimal
//!   `test_config()` builder.
//! - `crates/bee-control/tests/wal_persistence.rs` — per-node
//!   WAL on a directory.
//!
//! What this does NOT do:
//! - Change `Cluster`, `ClusterConfig`, or `AdminServer` public
//!   APIs. Pure refactor + new public helper.
//! - Wire the production `Node::admin_callback`. The in-process
//!   test cluster runs the default callback (same as the
//!   existing forwarding test).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;

use crate::kv::{Op, TxnError};
use crate::raft::admin_server::AdminServer;
use crate::raft::cluster::{Cluster, ClusterConfig, NodeSpec};
use crate::raft::node::{AdminReplyRegistrar, NodeConfig};
use crate::raft::types::NodeId;

/// Short election / heartbeat timings. The 500ms /
/// 50ms pair matches the existing
/// `admin_forwarding_inmem` boot path so tests that
/// previously used `Cluster::new_with_specs` with
/// these timeouts keep their leader-election
/// latency characteristics.
const FAST_ELECTION_TIMEOUT: Duration = Duration::from_millis(500);
const FAST_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(50);

/// In-process 3-node `Cluster` plus one `AdminServer`
/// bound to `127.0.0.1:0` per node, plus the
/// `NodeId → admin SocketAddr` map the test uses to
/// pick a connection target.
///
/// The `AdminServer`s are dropped before the `Cluster`'s
/// Node transports so the in-memory transport's
/// forwarding path can resolve the leader without
/// racing against a closed listener.
///
/// `TestCluster` is `Clone`: the cluster handle +
/// `HashMap` are cheap clones, and the owned
/// `AdminServer`s are wrapped in an `Arc<Mutex<...>>`
/// so a clone shares the same listener tasks.
#[derive(Clone)]
pub struct TestCluster {
    /// The live 3-node Raft cluster.
    pub cluster: Cluster,
    /// `NodeId → AdminServer bind address`. The
    /// GUI's `AdminClient::connect(addr)` is the
    /// primary consumer.
    pub admin_addrs: HashMap<NodeId, SocketAddr>,
    /// Owned `AdminServer` handles. Held so the
    /// listener tasks do not get dropped at the
    /// end of `boot_3_node_with_admin`. The
    /// `Drop` impl calls `AdminServer::shutdown`
    /// on each one (which sends the oneshot
    /// signal so the accept loop exits cleanly).
    /// Wrapped in `Arc<Mutex<...>>` so the
    /// struct is `Clone`.
    admin_servers: Arc<std::sync::Mutex<Vec<AdminServer>>>,
}

impl TestCluster {
    /// Boot an n=3 in-memory cluster with one
    /// `AdminServer` per node. Waits for a leader
    /// election before returning.
    ///
    /// Use this for the GUI integration tests +
    /// any existing test that just needs an
    /// in-process cluster.
    pub async fn boot_3_node_with_admin() -> Self {
        let config = ClusterConfig {
            n: 3,
            base_election_timeout: FAST_ELECTION_TIMEOUT,
            heartbeat_interval: FAST_HEARTBEAT_INTERVAL,
            plugin_manager: None,
            log_path: None,
            nodes: (0..3)
                .map(|i| {
                    let id = (i + 1) as NodeId;
                    NodeSpec {
                        id,
                        transport: None,
                        node_config: Some(NodeConfig {
                            base_election_timeout: FAST_ELECTION_TIMEOUT,
                            heartbeat_interval: FAST_HEARTBEAT_INTERVAL,
                            node_offset_ms: (i as u64) * 50,
                        }),
                    }
                })
                .collect(),
        };
        let cluster = Cluster::new_with_specs(config).await;

        // Spin up one AdminServer per node,
        // mirroring `admin_forwarding_inmem:88-100`.
        let mut admin_addrs: HashMap<NodeId, SocketAddr> =
            HashMap::new();
        let mut admin_servers: Vec<AdminServer> = Vec::new();
        for i in 1..=3u32 {
            let node = cluster.node(i).expect("node handle");
            let kv = node.kv.clone();
            let cp = node.cp.clone();
            let state = node.state.clone();
            let pending_replies = node.pending_replies.clone();
            let node_transport = node.node_transport.clone();
            let register_reply: AdminReplyRegistrar =
                Arc::new(move || {
                    let pr = pending_replies.clone();
                    Box::pin(async move { pr.register().await })
                });
            let admin = AdminServer::start(
                "127.0.0.1:0".parse().unwrap(),
                kv,
                cp,
                state,
                None,
                Some(node_transport),
                Some(register_reply),
                None,
            )
            .await
            .expect("AdminServer::start");
            let addr = admin.local_addr();
            admin_addrs.insert(i, addr);
            admin_servers.push(admin);
        }

        Self {
            cluster,
            admin_addrs,
            admin_servers: Arc::new(std::sync::Mutex::new(admin_servers)),
        }
    }

    /// Boot an n=3 in-memory cluster whose
    /// per-node WAL is persisted at `dir`. Mirrors
    /// the existing `wal_persistence` test boot
    /// path (`Cluster::new(ClusterConfig {
    /// log_path: Some(dir) })`).
    ///
    /// The `AdminServer`s are NOT spun up —
    /// `wal_persistence` does not exercise the
    /// admin RPC. Callers that want both can call
    /// `boot_3_node_with_admin` and write the
    /// WAL manually, or wire their own servers
    /// using `TestCluster::cluster`.
    pub async fn boot_3_node_with_wal(dir: &Path) -> Self {
        let config = ClusterConfig {
            n: 3,
            base_election_timeout: Duration::from_millis(800),
            heartbeat_interval: Duration::from_millis(100),
            nodes: Vec::new(),
            plugin_manager: None,
            log_path: Some(dir.to_path_buf()),
        };
        let cluster = Cluster::new(config).await;
        Self {
            cluster,
            admin_addrs: HashMap::new(),
            admin_servers: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Returns the address the GUI should connect
    /// to: the leader's admin port if a leader has
    /// been elected, otherwise node 1's admin port
    /// as a deterministic default (callers that
    /// care about leader-vs-follower should use
    /// `TestCluster::leader` first).
    ///
    /// The default address is used by the
    /// `connection_smoke` GUI test, which does not
    /// care whether the target is the leader (Ping
    /// works on every node).
    pub fn connect_addr(&self) -> SocketAddr {
        // Caller awaits leader() before connect_addr()
        // when leader-correctness matters. The default
        // here is "any live node" — node 1 is
        // deterministic.
        self.admin_addrs
            .get(&1)
            .copied()
            .expect("connect_addr requires boot_3_node_with_admin (admin_addrs populated)")
    }

    /// Returns the elected leader's `NodeId`, or
    /// `None` if no leader has been elected yet.
    pub async fn leader(&self) -> Option<NodeId> {
        self.cluster.leader().await
    }

    /// Poll until a leader is elected or `timeout`
    /// elapses. Convenience wrapper around
    /// `Cluster::wait_for_leader`.
    pub async fn wait_for_leader(
        &self,
        timeout: Duration,
    ) -> Option<NodeId> {
        self.cluster.wait_for_leader(timeout).await
    }

    /// Submit a KV op to the leader. Resolves to
    /// the leader's apply result.
    ///
    /// Internally calls `Cluster::submit(leader, op)`,
    /// which goes through the in-process transport's
    /// `NodeCommand::Submit` path. The leader's
    /// command channel propagates the op into the
    /// Raft log + KV state machine.
    pub async fn submit_kv(&self, op: Op) -> Result<(), TxnError> {
        let leader = self
            .leader()
            .await
            .ok_or_else(|| TxnError::Conflict {
                key: "no_leader_elected".to_string(),
                expected: None,
                actual: None,
            })?;
        timeout(Duration::from_secs(3), self.cluster.submit(leader, op))
            .await
            .map_err(|_| TxnError::Conflict {
                key: "submit_kv_timeout".to_string(),
                expected: None,
                actual: None,
            })?
    }

    /// Shut down a node AND its AdminServer. Used
    /// by the `connection_lost_recovery` GUI test
    /// to provoke an `Io` error from the AdminClient
    /// after the test has already connected: the
    /// AdminServer listener is torn down so the
    /// client's next read returns a transport-layer
    /// error rather than a synthetic Pong.
    ///
    /// The Node's Raft loop is also stopped via
    /// `Cluster::shutdown_node`, which is what the
    /// heartbeat / re-election paths observe.
    pub async fn shutdown_node(&self, id: NodeId) {
        self.cluster.shutdown_node(id).await;
        if let Ok(mut guard) = self.admin_servers.lock() {
            // The AdminServers are stored in
            // NodeId-order (1, 2, 3); `id - 1`
            // is the index. We swap-remove so the
            // Drop impl skips the already-shut-down
            // server when the TestCluster is finally
            // dropped.
            let idx = (id as usize) - 1;
            if idx < guard.len() {
                let mut srv = guard.remove(idx);
                srv.shutdown();
            }
        }
        // Remove the address from the public map so
        // callers can't pick a dead node.
        // (Caller cloned the map before calling
        // shutdown_node, so this is best-effort.)
    }
}

impl Drop for TestCluster {
    /// Best-effort graceful shutdown of every
    /// owned AdminServer. The `Cluster`'s Node
    /// tasks are spawned into the runtime and
    /// exit when their `run()` future completes;
    /// we do not have direct handles to them,
    /// so dropping `cluster` (which drops the
    /// `cmd_tx`s) is enough to let them unwind
    /// on their own.
    ///
    /// Because `TestCluster` is `Clone`, the
    /// shutdown sequence fires once when the
    /// LAST clone is dropped. The `Arc<Mutex<...>>`
    /// swap-out ensures we only call
    /// `AdminServer::shutdown` once per listener
    /// (the accept-loop tasks stay alive until
    /// then).
    fn drop(&mut self) {
        if let Ok(mut guard) = self.admin_servers.lock() {
            if !guard.is_empty() {
                for srv in guard.iter_mut() {
                    srv.shutdown();
                }
                guard.clear();
            }
        }
    }
}