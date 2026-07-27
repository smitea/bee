//! S17 §2: Deployer wiring for signature-driven Producer/Subscriber detection.
//!
//! These 5 tests exercise the CP-level ops that the deployer relies on
//! for its Producer/Subscriber decision. The end-to-end deployer test
//! (calling `Deployer::deploy(pipeline)` with a real `Pipeline` built
//! from SQL) is added once `Pipeline::from_sql` lands in §2.

use std::time::Duration;

use bee_control::kv::Op;
use bee_control::raft::cluster::{Cluster, ClusterConfig};

async fn fresh_cluster() -> Cluster {
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster
        .wait_for_leader(Duration::from_secs(5))
        .await
        .expect("leader");
    cluster
}

async fn read_producer(cluster: &Cluster, sig: &str) -> Option<u32> {
    for (id, _handle) in cluster.nodes() {
        if !cluster.is_alive(id) {
            continue;
        }
        let handle = cluster.node(id).expect("handle");
        let cp = handle.cp.lock().await;
        if let Some(p) = cp.lookup_datasource_producer(sig) {
            return Some(p);
        }
    }
    None
}

async fn read_count(cluster: &Cluster) -> usize {
    for (id, _handle) in cluster.nodes() {
        if !cluster.is_alive(id) {
            continue;
        }
        let handle = cluster.node(id).expect("handle");
        let cp = handle.cp.lock().await;
        return cp.datasource_producer_count();
    }
    0
}

async fn wait_for_count(cluster: &Cluster, expected: usize, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if read_count(cluster).await == expected {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("count did not reach {expected} within {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_first_pipeline_becomes_producer() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "test:binance:subscribe:abc123".to_string();
    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig.clone(),
                job_id: 1,
            },
        )
        .await
        .expect("register producer");
    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;
    assert_eq!(read_producer(&cluster, &sig).await, Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_second_pipeline_with_same_signature_becomes_subscriber() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "test:binance:subscribe:def456".to_string();

    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig.clone(),
                job_id: 1,
            },
        )
        .await
        .expect("register producer 1");
    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 2,
                dag_hash: "d".into(),
                owner_node: leader,
                tenant: 0,
                dependencies: vec![],
                plugins: vec![],
            },
        )
        .await
        .expect("register job 2");
    cluster
        .submit(
            leader,
            Op::RegisterDependency {
                downstream_job: 2,
                upstream_job: 1,
                stream: sig.clone(),
            },
        )
        .await
        .expect("register dep 2->1");

    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;
    assert_eq!(
        read_producer(&cluster, &sig).await,
        Some(1),
        "Job 1 is still the sole Producer"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_pipeline_with_different_args_gets_different_producer() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig_btc = "test:binance:subscribe:BTC".to_string();
    let sig_eth = "test:binance:subscribe:ETH".to_string();

    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig_btc.clone(),
                job_id: 1,
            },
        )
        .await
        .expect("register BTC");
    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig_eth.clone(),
                job_id: 2,
            },
        )
        .await
        .expect("register ETH");

    wait_for_count(&cluster, 2, Duration::from_secs(2)).await;
    assert_eq!(read_producer(&cluster, &sig_btc).await, Some(1));
    assert_eq!(read_producer(&cluster, &sig_eth).await, Some(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_repeat_of_same_signature_is_idempotent() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "test:replay:sig".to_string();

    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer {
                signature: sig.clone(),
                job_id: 1,
            },
        )
        .await
        .expect("register 1");
    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;

    // Re-register the same Job (idempotent at CP) and add a
    // self-dependency on its own stream. The producer entry should
    // still point at Job 1; the count stays at 1.
    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 1,
                dag_hash: "d".into(),
                owner_node: leader,
                tenant: 0,
                dependencies: vec![],
                plugins: vec![],
            },
        )
        .await
        .expect("re-register job 1");
    cluster
        .submit(
            leader,
            Op::RegisterDependency {
                downstream_job: 1,
                upstream_job: 1,
                stream: sig.clone(),
            },
        )
        .await
        .expect("self dep");

    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;
    assert_eq!(read_producer(&cluster, &sig).await, Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_multiple_pipelines_only_first_is_producer() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "test:multi:sig".to_string();

    for job_id in 1u32..=3 {
        if job_id == 1 {
            cluster
                .submit(
                    leader,
                    Op::RegisterDatasourceProducer {
                        signature: sig.clone(),
                        job_id,
                    },
                )
                .await
                .expect("register producer 1");
        } else {
            cluster
                .submit(
                    leader,
                    Op::RegisterJob {
                        job_id,
                        dag_hash: "d".into(),
                        owner_node: leader,
                        tenant: 0,
                        dependencies: vec![],
                        plugins: vec![],
                    },
                )
                .await
                .expect("register job");
            cluster
                .submit(
                    leader,
                    Op::RegisterDependency {
                        downstream_job: job_id,
                        upstream_job: 1,
                        stream: sig.clone(),
                    },
                )
                .await
                .expect("register dep");
        }
    }

    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;
    assert_eq!(read_producer(&cluster, &sig).await, Some(1));
}

// ---- S17 §2: end-to-end Deployer tests (SQL → Pipeline → CP) ----

use bee_control::deployer::{Deployer, DeployerConfig, Pipeline};
use bee_control::control_plane::JobMode;

/// Wait until the producer count on the deployer's leader CP reaches
/// `expected`. Used by the end-to-end tests as a barrier between
/// `Deployer::deploy` calls so the second deploy's
/// `lookup_datasource_producer` observes the first deploy's
/// `RegisterDatasourceProducer` apply.
async fn wait_for_producer_count_on_deployer(
    deployer: &Deployer,
    expected: usize,
    timeout: Duration,
) {
    let leader = deployer
        .cluster
        .wait_for_leader(Duration::from_secs(2))
        .await
        .expect("leader for wait_for_producer_count_on_deployer");
    let handle = deployer
        .cluster
        .node(leader)
        .expect("leader handle exists");
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        {
            let cp = handle.cp.lock().await;
            if cp.datasource_producer_count() == expected {
                return;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "producer count on deployer's leader did not reach {expected} within {timeout:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Snapshot a Job's [`JobMode`] from the leader's CP.
async fn job_mode_on_deployer(deployer: &Deployer, job_id: u32) -> JobMode {
    let leader = deployer
        .cluster
        .leader()
        .await
        .expect("leader for job_mode_on_deployer");
    let handle = deployer
        .cluster
        .node(leader)
        .expect("leader handle exists");
    let cp = handle.cp.lock().await;
    cp.job_mode(job_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn end_to_end_deployer_registers_producer_for_new_signature() {
    let pipeline = Pipeline::from_sql(
        "quant-btc",
        "use binance;\nSELECT * FROM binance.subscribe(symbol='BTC/USDT', interval='5min')",
    )
    .expect("parse pipeline");
    assert!(
        !pipeline.stream_identities.is_empty(),
        "pipeline must have stream identities for this test to mean anything"
    );

    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let job_id = deployer.deploy(pipeline).await.expect("deploy");

    wait_for_producer_count_on_deployer(&deployer, 1, Duration::from_secs(3)).await;
    assert_eq!(
        job_mode_on_deployer(&deployer, job_id).await,
        JobMode::Producer,
        "first deploy of a signature must be the Producer",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn end_to_end_deployer_second_pipeline_becomes_subscriber() {
    let sql = "use binance;\nSELECT * FROM binance.subscribe(symbol='BTC/USDT', interval='5min')";
    let p1 = Pipeline::from_sql("p1", sql).expect("p1");
    let p2 = Pipeline::from_sql("p2", sql).expect("p2");

    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let job_a = deployer.deploy(p1).await.expect("deploy A");
    // Wait for A's Producer registration to be applied on the
    // leader's CP before B looks it up. Without this barrier, B may
    // run its lookup before A's apply path has committed, and
    // become a (second) Producer instead of a Subscriber.
    wait_for_producer_count_on_deployer(&deployer, 1, Duration::from_secs(3)).await;
    let job_b = deployer.deploy(p2).await.expect("deploy B");

    // The CP registry must still contain exactly 1 Producer
    // (first writer wins).
    wait_for_producer_count_on_deployer(&deployer, 1, Duration::from_secs(3)).await;
    assert_eq!(
        job_mode_on_deployer(&deployer, job_a).await,
        JobMode::Producer,
        "A is the canonical owner of the stream",
    );
    assert_eq!(
        job_mode_on_deployer(&deployer, job_b).await,
        JobMode::Subscriber,
        "B reuses A's stream and must be tagged as a Subscriber",
    );
}
