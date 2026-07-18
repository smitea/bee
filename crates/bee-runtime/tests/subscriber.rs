use std::time::Duration;

use bee_control::control_plane::JobMode;
use bee_control::kv::Op;
use bee_control::raft::cluster::{Cluster, ClusterConfig};
use bee_runtime::subscriber::{StreamSubscriber, SubscriberState};
use bee_types::JobLifecycleState;

async fn fresh_cluster() -> Cluster {
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster.wait_for_leader(Duration::from_secs(5)).await.expect("leader");
    cluster
}

/// Poll the CP until `job_id`'s lifecycle equals `expected`, or timeout.
async fn wait_for_lifecycle(
    cluster: &Cluster,
    leader: u32,
    job_id: u32,
    expected: JobLifecycleState,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let lifecycle = {
            let handle = cluster.node(leader).expect("handle");
            let cp = handle.cp.lock().await;
            cp.list_jobs()
                .into_iter()
                .find(|j| j.job_id == job_id)
                .map(|j| j.lifecycle)
        };
        if lifecycle == Some(expected) {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timeout waiting for job {job_id} lifecycle == {expected:?}; got {lifecycle:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscriber_state_machine_drives_off_real_cp_lifecycle() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    // Job 1: Producer. Register both the DatasourceProducer entry AND
    // the JobRecord so the subscriber test can read Job 1's lifecycle
    // via `list_jobs()`.
    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: "sig".into(), job_id: 1 })
        .await.expect("register producer 1");
    cluster.submit(leader, Op::RegisterJob { job_id: 1, dag_hash: "d".into(), owner_node: leader, tenant: 0, dependencies: vec![] })
        .await.expect("register job 1");
    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 1, state: JobLifecycleState::Running })
        .await.expect("job 1 -> Running");

    // Job 2: Subscriber. Wire the dep AFTER Job 2 is Running (S18
    // auto-flip pattern), then set Running again.
    cluster.submit(leader, Op::RegisterJob { job_id: 2, dag_hash: "d".into(), owner_node: leader, tenant: 0, dependencies: vec![] })
        .await.expect("register job 2");
    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 2, state: JobLifecycleState::Running })
        .await.expect("job 2 -> Running");
    cluster.submit(leader, Op::RegisterDependency { downstream_job: 2, upstream_job: 1, stream: "sig".into() })
        .await.expect("register dep 2 -> 1");
    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 2, state: JobLifecycleState::Running })
        .await.expect("job 2 -> Running (post-dep)");

    assert!(cluster.wait_for_cp_converge(2, 0, Duration::from_secs(2)).await);
    wait_for_lifecycle(&cluster, leader, 1, JobLifecycleState::Running, Duration::from_secs(2)).await;

    let mut sub = StreamSubscriber::new(1, "sig".into());
    assert_eq!(sub.state, SubscriberState::Connecting);

    let lifecycle = {
        let handle = cluster.node(leader).expect("handle");
        let cp = handle.cp.lock().await;
        cp.list_jobs().into_iter().find(|j| j.job_id == 1).expect("A exists").lifecycle
    };
    let t = sub.tick(lifecycle, true);
    assert_eq!(t.next, SubscriberState::Active);

    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 1, state: JobLifecycleState::Failed })
        .await.expect("job 1 -> Failed");
    wait_for_lifecycle(&cluster, leader, 1, JobLifecycleState::Failed, Duration::from_secs(2)).await;
    let lifecycle = {
        let handle = cluster.node(leader).expect("handle");
        let cp = handle.cp.lock().await;
        cp.list_jobs().into_iter().find(|j| j.job_id == 1).expect("A exists").lifecycle
};
    let t = sub.tick(lifecycle, true);
    assert_eq!(t.next, SubscriberState::WaitingForUpstream);

    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 1, state: JobLifecycleState::Running })
        .await.expect("job 1 -> Running (revive)");
    wait_for_lifecycle(&cluster, leader, 1, JobLifecycleState::Running, Duration::from_secs(2)).await;
    let lifecycle = {
        let handle = cluster.node(leader).expect("handle");
        let cp = handle.cp.lock().await;
        cp.list_jobs().into_iter().find(|j| j.job_id == 1).expect("A exists").lifecycle
    };
    let t = sub.tick(lifecycle, true);
    assert_eq!(t.next, SubscriberState::Resubscribing);
    let t = sub.tick(lifecycle, true);
    assert_eq!(t.next, SubscriberState::Active);

    let handle = cluster.node(leader).expect("handle");
    let cp = handle.cp.lock().await;
    assert_eq!(cp.job_mode(1), JobMode::Producer);
    assert_eq!(cp.job_mode(2), JobMode::Subscriber);
}
