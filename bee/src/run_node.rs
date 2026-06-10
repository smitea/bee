//! S33.1: `bee node` subcommand — spawn a single Raft
//! Node with `TcpTransport`, listening on `--bind`,
//! dialing each `--peer ID=ADDR`. The Node runs until
//! SIGTERM/SIGINT.
//!
//! This is the production entry point. The existing
//! in-process `bee jobs` / `bee cluster` CLIs continue
//! to work (they spin up a 3-node in-memory cluster as
//! a demo); `bee node` is for real multi-process
//! deployments.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bee_control::raft::{Node, NodeConfig, NodeId, NodeTransport, TcpTransport};
use bee_control::{ControlPlaneStateMachine, KVStateMachine};
use tokio::signal;
use tokio::sync::Mutex;

/// A handle to a running `bee node` instance. Used by
/// the S33.2 admin client to push `NodeCommand`s into
/// the Node's transport. For the MVP, this is
/// unused (the Node runs until SIGTERM, then exits).
#[allow(dead_code)]
pub struct NodeHandle {
    pub id: NodeId,
    pub bind_addr: SocketAddr,
    pub kv: Arc<Mutex<KVStateMachine>>,
    pub cp: Arc<Mutex<ControlPlaneStateMachine>>,
}

pub fn parse_peers(args: &[String]) -> Result<Vec<(NodeId, SocketAddr)>, String> {
    let mut peers: Vec<(NodeId, SocketAddr)> = Vec::new();
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == "--peer" {
            let spec = iter
                .next()
                .ok_or_else(|| "--peer requires ID=ADDR".to_string())?;
            let (id_str, addr_str) = spec
                .split_once('=')
                .ok_or_else(|| format!("--peer must be ID=ADDR, got `{spec}`"))?;
            let id: NodeId = id_str
                .parse()
                .map_err(|e| format!("invalid peer id `{id_str}`: {e}"))?;
            let addr: SocketAddr = addr_str
                .parse()
                .map_err(|e| format!("invalid peer addr `{addr_str}`: {e}"))?;
            peers.push((id, addr));
        }
    }
    Ok(peers)
}

pub async fn run_node(args: Vec<String>) -> Result<(), String> {
    // Parse: --id N --bind ADDR [--peer ID=ADDR ...]
    //       [--base-election-timeout MS] [--heartbeat-interval MS]
    //       [--node-offset MS]
    let mut id: Option<NodeId> = None;
    let mut bind: Option<SocketAddr> = None;
    let mut base_election_timeout_ms: u64 = 800;
    let mut heartbeat_interval_ms: u64 = 100;
    let mut node_offset_ms: u64 = 0;
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--id" => {
                id = Some(
                    iter.next()
                        .ok_or_else(|| "--id requires an argument".to_string())?
                        .parse()
                        .map_err(|e| format!("invalid id: {e}"))?,
                );
            }
            "--bind" => {
                bind = Some(
                    iter.next()
                        .ok_or_else(|| "--bind requires an argument".to_string())?
                        .parse()
                        .map_err(|e| format!("invalid bind: {e}"))?,
                );
            }
            "--base-election-timeout" => {
                base_election_timeout_ms = iter
                    .next()
                    .ok_or_else(|| "--base-election-timeout requires an argument".to_string())?
                    .parse()
                    .map_err(|e| format!("invalid base-election-timeout: {e}"))?;
            }
            "--heartbeat-interval" => {
                heartbeat_interval_ms = iter
                    .next()
                    .ok_or_else(|| "--heartbeat-interval requires an argument".to_string())?
                    .parse()
                    .map_err(|e| format!("invalid heartbeat-interval: {e}"))?;
            }
            "--node-offset" => {
                node_offset_ms = iter
                    .next()
                    .ok_or_else(|| "--node-offset requires an argument".to_string())?
                    .parse()
                    .map_err(|e| format!("invalid node-offset: {e}"))?;
            }
            "--peer" => {
                // Consumed in parse_peers below.
                iter.next();
            }
            other => return Err(format!("unknown flag `{other}`")),
        }
    }
    let id = id.ok_or_else(|| "--id is required".to_string())?;
    let bind = bind.ok_or_else(|| "--bind is required".to_string())?;
    let peers = parse_peers(&args)?;

    // Build the TcpTransport. The bind happens here
    // (synchronously, awaited); connect_peers happens
    // next with retry on transient ECONNREFUSED.
    let tcp = TcpTransport::bind(id, bind.to_string())
        .await
        .map_err(|e| format!("bind {bind}: {e}"))?;
    let peer_addrs: Vec<(NodeId, String)> = peers
        .iter()
        .map(|(pid, addr)| (*pid, addr.to_string()))
        .collect();
    tcp.connect_peers(peer_addrs)
        .await
        .map_err(|e| format!("connect_peers: {e}"))?;

    // Build the Node. The kv/cp state machines live
    // in this process (the MVP does not replicate
    // them via the KV cluster — that's S33.2).
    let kv = Arc::new(Mutex::new(KVStateMachine::new()));
    let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new()));
    let peer_ids: Vec<NodeId> = peers.iter().map(|(pid, _)| *pid).collect();
    let node_config = NodeConfig {
        base_election_timeout: Duration::from_millis(base_election_timeout_ms),
        heartbeat_interval: Duration::from_millis(heartbeat_interval_ms),
        node_offset_ms,
    };
    let transport_arc: Arc<dyn NodeTransport> = Arc::new(tcp);
    let node = Node::new(
        id,
        peer_ids,
        transport_arc,
        kv.clone(),
        cp.clone(),
        node_config,
    );
    let task = tokio::spawn(async move {
        let _ = node.run().await;
    });

    println!("bee node {id} listening on {bind} (peers: {peers:?})");
    // Block on SIGTERM / SIGINT, then drop the
    // transport. Node::run returns when its
    // transport is dropped.
    let mut term = signal::unix::signal(signal::unix::SignalKind::terminate())
        .map_err(|e| format!("install SIGTERM: {e}"))?;
    tokio::select! {
        _ = signal::ctrl_c() => {
            println!("SIGINT received, shutting down node {id}");
        }
        _ = term.recv() => {
            println!("SIGTERM received, shutting down node {id}");
        }
    }
    task.abort();
    let _ = NodeHandle {
        id,
        bind_addr: bind,
        kv,
        cp,
    };
    Ok(())
}
