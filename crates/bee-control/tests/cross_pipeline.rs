//! S18: Cross-Pipeline edges — dependency tracking + lifecycle gating.
//!
//! When a Job declares a dependency on an upstream Job's output stream,
//! the downstream Job must not enter `Running` until the upstream
//! reaches `Running`. The ControlPlane SM tracks the dependency list
//! per Job and the orchestrator's tick re-evaluates lifecycles on
//! upstream state changes.
//!
//! The MVP for S18 is the metadata layer: dependency registration,
//! `job_dependencies_satisfied`, `evaluate_job_state`, and the
//! `downstream_jobs_of` lookup. The actual data-channel resolution
//! (in-process vs BRP) and cross-Node rebalance on upstream kill
//! are wired in S18+ follow-ups (S25, S09 forwarder reuse).

use std::time::Duration;

use bee_control::kv::{JobLifecycleState, Op};
use bee_control::raft::cluster::{Cluster, ClusterConfig};

async fn fresh_cluster() -> Cluster {
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster
        .wait_for_leader(Duration::from_secs(5))
        .await
        .expect("leader elected");
    cluster
}

/// Helper: read the lifecycle of `job_id` from any alive node.
async fn read_lifecycle(cluster: &Cluster, job_id: u32) -> Option<JobLifecycleState> {
    for (id, handle) in cluster.nodes() {
        if !cluster.is_alive(id) {
            continue;
        }
        let cp = handle.cp.lock().await;
        if let Some(j) = cp.get_job(job_id) {
            return Some(j.lifecycle);
        }
    }
    None
}

/// Helper: read whether `job_id` has the given dependency recorded.
async fn read_has_dependency(
    cluster: &Cluster,
    job_id: u32,
    upstream: u32,
    stream: &str,
) -> bool {
    for (id, handle) in cluster.nodes() {
        if !cluster.is_alive(id) {
            continue;
        }
        let cp = handle.cp.lock().await;
        if let Some(j) = cp.get_job(job_id) {
            return j
                .dependencies
                .iter()
                .any(|d| d.upstream_job == upstream && d.stream == stream);
        }
    }
    false
}

async fn wait_for_lifecycle(
    cluster: &Cluster,
    job_id: u32,
    expected: JobLifecycleState,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if read_lifecycle(cluster, job_id).await == Some(expected) {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "job {job_id} did not reach lifecycle {expected:?} within {timeout:?}; current: {:?}",
                read_lifecycle(cluster, job_id).await
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_with_no_deps_starts_pending_evaluates_to_running() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 1,
                dag_hash: "h".into(),
                owner_node: 1, tenant: 0, dependencies: vec![] },
        )
        .await
        .unwrap();

    // Lifecycle is initially Pending
    wait_for_lifecycle(&cluster, 1, JobLifecycleState::Pending, Duration::from_secs(2))
        .await;

    // evaluate_job_state says Running (no deps)
    let (_, handle) = cluster.nodes().next().unwrap();
    let cp = handle.cp.lock().await;
    assert_eq!(cp.evaluate_job_state(1), JobLifecycleState::Running);
    assert!(cp.job_dependencies_satisfied(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downstream_with_unsatisfied_dep_evaluates_to_waiting() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    // Upstream A
    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 1,
                dag_hash: "a".into(),
                owner_node: 1, tenant: 0, dependencies: vec![] },
        )
        .await
        .unwrap();
    // Downstream B with dep on A
    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 2,
                dag_hash: "b".into(),
                owner_node: 1, tenant: 0, dependencies: vec![] },
        )
        .await
        .unwrap();
    cluster
        .submit(
            leader,
            Op::RegisterDependency {
                downstream_job: 2,
                upstream_job: 1,
                stream: "output".into(),
            },
        )
        .await
        .unwrap();

    // Wait for the dep to be visible
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if read_has_dependency(&cluster, 2, 1, "output").await {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("dependency not recorded in 2s");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // B has a dep on A which is Pending → B's expected state is
    // WaitingForUpstream.
    let (_, handle) = cluster.nodes().next().unwrap();
    let cp = handle.cp.lock().await;
    assert_eq!(cp.evaluate_job_state(2), JobLifecycleState::WaitingForUpstream);
    assert!(!cp.job_dependencies_satisfied(2));
    assert!(cp.downstream_jobs_of(1).contains(&2));
    assert!(cp.downstream_jobs_of(99).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upstream_running_promotes_downstream_evaluation_to_running() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 1,
                dag_hash: "a".into(),
                owner_node: 1, tenant: 0, dependencies: vec![] },
        )
        .await
        .unwrap();
    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 2,
                dag_hash: "b".into(),
                owner_node: 1, tenant: 0,     dependencies: Vec::new(),
},
        )
        .await
        .unwrap();
    cluster
        .submit(
            leader,
            Op::RegisterDependency {
                downstream_job: 2,
                upstream_job: 1,
                stream: "output".into(),
            },
        )
        .await
        .unwrap();

    // Initially B is WaitingForUpstream (the RegisterDependency
    // demotes Pending → WaitingForUpstream).
    wait_for_lifecycle(
        &cluster,
        2,
        JobLifecycleState::WaitingForUpstream,
        Duration::from_secs(2),
    )
    .await;
    let (_, handle) = cluster.nodes().next().unwrap();
    {
        let cp = handle.cp.lock().await;
        assert_eq!(cp.evaluate_job_state(2), JobLifecycleState::WaitingForUpstream);
    }

    // Promote A to Running
    cluster
        .submit(
            leader,
            Op::UpdateJobLifecycle {
                job_id: 1,
                state: JobLifecycleState::Running,
            },
        )
        .await
        .unwrap();
    wait_for_lifecycle(&cluster, 1, JobLifecycleState::Running, Duration::from_secs(2))
        .await;

    // Now B's deps are satisfied — evaluate_job_state should be Running.
    let cp = handle.cp.lock().await;
    assert_eq!(cp.evaluate_job_state(2), JobLifecycleState::Running);
    assert!(cp.job_dependencies_satisfied(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adding_dep_to_running_job_demotes_it_to_waiting() {
    // Per the apply_op logic: RegisterDependency on a Running Job
    // demotes it to WaitingForUpstream. This guards against a Job
    // getting new deps while running (which would otherwise silently
    // miss the gating).
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 1,
                dag_hash: "a".into(),
                owner_node: 1, tenant: 0, dependencies: vec![] },
        )
        .await
        .unwrap();
    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 2,
                dag_hash: "b".into(),
                owner_node: 1, tenant: 0, dependencies: vec![] },
        )
        .await
        .unwrap();
    // A Running first
    cluster
        .submit(
            leader,
            Op::UpdateJobLifecycle {
                job_id: 1,
                state: JobLifecycleState::Running,
            },
        )
        .await
        .unwrap();
    wait_for_lifecycle(&cluster, 1, JobLifecycleState::Running, Duration::from_secs(2))
        .await;
    // B is Pending (no deps yet)
    {
        let (_, handle) = cluster.nodes().next().unwrap();
        let cp = handle.cp.lock().await;
        assert_eq!(cp.get_job(2).map(|j| j.lifecycle), Some(JobLifecycleState::Pending));
    }

    // Now add a dep on a third Job 3 that does not exist
    cluster
        .submit(
            leader,
            Op::RegisterDependency {
                downstream_job: 2,
                upstream_job: 3,
                stream: "x".into(),
            },
        )
        .await
        .unwrap();

    let (_, handle) = cluster.nodes().next().unwrap();
    let cp = handle.cp.lock().await;
    // B's lifecycle should be WaitingForUpstream now (upstream 3 is
    // not Running, in fact not registered at all).
    assert_eq!(cp.get_job(2).map(|j| j.lifecycle), Some(JobLifecycleState::WaitingForUpstream));
    assert_eq!(cp.evaluate_job_state(2), JobLifecycleState::WaitingForUpstream);
}
