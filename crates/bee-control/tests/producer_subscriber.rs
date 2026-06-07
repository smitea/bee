//! S17: DatasourceSignature (hash) + Producer/Subscriber detection.
//!
//! Two deploys of the same Datasource (same `signature`) must result
//! in exactly ONE Producer and N Subscribers. The first writer wins:
//! subsequent `Op::RegisterDatasourceProducer { sig, job_id }` ops for
//! a known `sig` are no-ops at the CP level (the existing entry is
//! preserved), and the deployer can then read the CP to find the
//! Producer's JobId.

use std::time::Duration;

use bee_control::kv::Op;
use bee_control::raft::cluster::{Cluster, ClusterConfig};

/// Read the ControlPlane from any alive node, returning the
/// DatasourceProducer entry for `sig` if present.
async fn read_producer(cluster: &Cluster, sig: &str) -> Option<u32> {
    for (id, handle) in cluster.nodes() {
        if !cluster.is_alive(id) {
            continue;
        }
        let cp = handle.cp.lock().await;
        if let Some(producer) = cp.lookup_datasource_producer(sig) {
            return Some(producer);
        }
    }
    None
}

async fn read_producer_count(cluster: &Cluster) -> usize {
    for (id, handle) in cluster.nodes() {
        if !cluster.is_alive(id) {
            continue;
        }
        let cp = handle.cp.lock().await;
        return cp.datasource_producer_count();
    }
    0
}

async fn wait_for_producer(cluster: &Cluster, sig: &str, expected: u32, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if read_producer(cluster, sig).await == Some(expected) {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "signature {sig:?} did not resolve to producer {expected} within {timeout:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_count(cluster: &Cluster, expected: usize, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if read_producer_count(cluster).await == expected {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "datasource producer count did not reach {expected} within {timeout:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn fresh_cluster() -> Cluster {
    let cfg = ClusterConfig::default();
    let cluster = Cluster::new(cfg).await;
    cluster
        .wait_for_leader(Duration::from_secs(5))
        .await
        .expect("leader elected");
    cluster
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_register_datasource_producer_creates_entry() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "abc123".to_string();

    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig.clone(),
                job_id: 1,
            },
        )
        .await
        .expect("submit");

    wait_for_producer(&cluster, &sig, 1, Duration::from_secs(2)).await;
    assert_eq!(read_producer_count(&cluster).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_register_with_same_signature_is_idempotent_first_writer_wins() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "binance:BTC/USDT:5m".to_string();

    // Job 1: first deploy with this Datasource → becomes Producer
    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig.clone(),
                job_id: 1,
            },
        )
        .await
        .expect("submit 1");
    wait_for_producer(&cluster, &sig, 1, Duration::from_secs(2)).await;

    // Job 2: second deploy with the same Datasource → should be a no-op
    // at the CP level. The deployer reads lookup() == Some(1) and
    // knows this Job is a Subscriber pointing at Job 1.
    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig.clone(),
                job_id: 2,
            },
        )
        .await
        .expect("submit 2");

    // After the second submit, the Producer is still Job 1 — total
    // producer count is 1, not 2.
    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;
    assert_eq!(read_producer(&cluster, &sig).await, Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn different_signatures_get_different_producers() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig_a = "binance:BTC/USDT:5m".to_string();
    let sig_b = "binance:ETH/USDT:5m".to_string();

    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig_a.clone(),
                job_id: 1,
            },
        )
        .await
        .expect("submit A");
    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig_b.clone(),
                job_id: 2,
            },
        )
        .await
        .expect("submit B");
    wait_for_count(&cluster, 2, Duration::from_secs(2)).await;

    assert_eq!(read_producer(&cluster, &sig_a).await, Some(1));
    assert_eq!(read_producer(&cluster, &sig_b).await, Some(2));
    assert_eq!(read_producer_count(&cluster).await, 2);
}
