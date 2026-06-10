//! End-to-end TCP integration tests for the 3-node
//! `Cluster` with `TcpTransport`. Boots 3 `Node`s on
//! `127.0.0.1:0` (random port), waits for leader
//! election, simulates a process crash, asserts
//! re-election, then teardown.
//!
//! All three test fns are `#[tokio::test]`; the test
//! runtime is the multi-thread flavor (so a crash in
//! one Node doesn't block the others).
//!
//! See ADR-0010 + S33.1 design spec
//! `docs/superpowers/specs/2026-06-10-s33-1-multinode-cluster-failover-design.md`.

use std::net::SocketAddr;
use std::time::Duration;

use crate::raft::{
    Cluster, ClusterConfig, NodeConfig, NodeSpec, NodeTransportSpec,
};

/// Build a 3-node `Cluster` with `TcpTransport` for
/// each slot, listening on `127.0.0.1:0` (random port).
async fn boot_tcp_3_node() -> (Cluster, Vec<SocketAddr>) {
    // Pick 3 random free ports. We bind a
    // TcpListener to claim a port, then drop it;
    // the small race (port reuse) is fine for a
    // test in a single CI process.
    async fn pick_port() -> u16 {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind 127.0.0.1:0");
        let p = l.local_addr().expect("local_addr").port();
        drop(l);
        p
    }
    let ports = [pick_port().await, pick_port().await, pick_port().await];
    let addrs: Vec<SocketAddr> = ports
        .iter()
        .map(|p| format!("127.0.0.1:{p}").parse().unwrap())
        .collect();

    // Each Node's "peers" = the OTHER two addrs.
    let mut specs = Vec::new();
    for i in 0..3 {
        let id = (i + 1) as u32;
        let peers: Vec<(u32, SocketAddr)> = (0..3)
            .filter(|&j| j != i)
            .map(|j| ((j + 1) as u32, addrs[j]))
            .collect();
        specs.push(NodeSpec {
            id,
            transport: Some(NodeTransportSpec::Tcp {
                bind_addr: addrs[i],
                peers,
            }),
            node_config: Some(NodeConfig {
                base_election_timeout: Duration::from_millis(500),
                heartbeat_interval: Duration::from_millis(50),
                node_offset_ms: (i as u64) * 50,
            }),
        });
    }

    let config = ClusterConfig {
        n: 3,
        base_election_timeout: Duration::from_millis(500),
        heartbeat_interval: Duration::from_millis(50),
        nodes: specs,
    };
    let cluster = Cluster::new_with_specs(config).await;
    (cluster, addrs)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcp_3_node_elects_leader() {
    let (cluster, _addrs) = boot_tcp_3_node().await;
    let leader_id = cluster
        .wait_for_leader(Duration::from_secs(5))
        .await
        .expect("leader not elected within 5s");
    assert!(
        leader_id >= 1 && leader_id <= 3,
        "leader id out of range: {leader_id}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcp_3_node_survives_simulated_crash() {
    let (cluster, _addrs) = boot_tcp_3_node().await;
    let leader_id = cluster
        .wait_for_leader(Duration::from_secs(5))
        .await
        .expect("leader not elected within 5s");
    // Pick the leader to "kill". For a 3-node cluster
    // a leader shutdown leaves 2 followers; quorum is
    // 2/3 so re-election is possible.
    let killed = leader_id;
    cluster.simulate_process_crash(killed).await;
    // Surviving 2 nodes must re-elect within 5s.
    let new_leader = cluster
        .wait_for_leader(Duration::from_secs(10))
        .await
        .expect("no leader after crash within 10s");
    assert_ne!(
        new_leader, killed,
        "the killed node is still the leader after shutdown"
    );
}
