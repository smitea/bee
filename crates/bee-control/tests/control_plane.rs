use std::time::Duration;

use bee_control::kv::TaskStatus;
use bee_control::{
    Cluster, ClusterConfig, ControlPlaneStateMachine, JobRecord, Op, TaskRecord,
};

fn test_config() -> ClusterConfig {
    ClusterConfig {
        n: 3,
        base_election_timeout: Duration::from_millis(800),
        heartbeat_interval: Duration::from_millis(100),
        nodes: Vec::new(), // in-memory default
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn register_job_on_node1_is_visible_on_node2_after_replication() {
    let cluster = Cluster::new(test_config()).await;
    let leader = cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader must be elected");

    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 42,
                dag_hash: "sha256:abc".to_string(),
                owner_node: 1, tenant: 0, },
        )
        .await
        .expect("submit must succeed");

    let converged = cluster
        .wait_for_cp_converge(1, 0, Duration::from_secs(2))
        .await;
    assert!(converged, "all nodes must converge on the new job");

    for id in 1..=3u32 {
        let cp = &cluster.node(id).expect("node").cp;
        let jobs: Vec<JobRecord> = cp.lock().await.list_jobs();
        assert!(
            jobs.iter().any(|j| j.job_id == 42 && j.dag_hash == "sha256:abc"),
            "node {id} must see job 42"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn register_task_and_update_status_replicate_linearly() {
    let cluster = Cluster::new(test_config()).await;
    let leader = cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader must be elected");

    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 1,
                dag_hash: "h1".to_string(),
                owner_node: 1, tenant: 0, },
        )
        .await
        .unwrap();

    cluster
        .submit(
            leader,
            Op::RegisterTask {
                task_id: 2,
                job_id: 1,
                phase_id: 1,
                owner_node: 2,
                status: TaskStatus::Pending,
                started_at_ms: 0,
            },
        )
        .await
        .unwrap();

    assert!(cluster.wait_for_cp_converge(1, 1, Duration::from_secs(2)).await);

    cluster
        .submit(
            leader,
            Op::UpdateTaskStatus {
                task_id: 2,
                new_status: TaskStatus::Running,
            },
        )
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let cp = &cluster.node(2).expect("node 2").cp;
        let task: Option<TaskRecord> = cp
            .lock()
            .await
            .list_tasks()
            .into_iter()
            .find(|t| t.task_id == 2);
        if let Some(t) = task {
            if t.status == TaskStatus::Running {
                break;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("task 2 did not reach Running on node 2 within 2s");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kv_and_control_plane_sms_coexist_without_interference() {
    let cluster = Cluster::new(test_config()).await;
    let leader = cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader must be elected");

    cluster
        .submit(
            leader,
            Op::Put {
                key: "state/task/t1/buf".to_string(),
                value: b"chunk-0".to_vec(),
            },
        )
        .await
        .unwrap();

    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 7,
                dag_hash: "h7".to_string(),
                owner_node: 1, tenant: 0, },
        )
        .await
        .unwrap();

    cluster
        .submit(
            leader,
            Op::RegisterTask {
                task_id: 71,
                job_id: 7,
                phase_id: 0,
                owner_node: 1,
                status: TaskStatus::Running,
                started_at_ms: 0,
            },
        )
        .await
        .unwrap();

    assert!(cluster
        .wait_for_log_converge("state/task/t1/buf", b"chunk-0", Duration::from_secs(2))
        .await);
    assert!(cluster.wait_for_cp_converge(1, 1, Duration::from_secs(2)).await);

    for id in 1..=3u32 {
        let kv = &cluster.node(id).expect("node").kv;
        assert_eq!(
            kv.lock().await.get("state/task/t1/buf"),
            Some(b"chunk-0".to_vec())
        );
        let cp = &cluster.node(id).expect("node").cp;
        assert_eq!(cp.lock().await.job_count(), 1);
        assert_eq!(cp.lock().await.task_count(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_status_for_unknown_task_is_a_silent_noop() {
    let cluster = Cluster::new(test_config()).await;
    let leader = cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader must be elected");

    let res = cluster
        .submit(
            leader,
            Op::UpdateTaskStatus {
                task_id: 999,
                new_status: TaskStatus::Running,
            },
        )
        .await;
    assert!(
        res.is_ok(),
        "Submit itself must succeed (Raft appends before applying)"
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let mut all_clean = true;
        for id in 1..=3u32 {
            let cp = &cluster.node(id).expect("node").cp;
            let tasks = cp.lock().await.list_tasks();
            if tasks.iter().any(|t| t.task_id == 999) {
                all_clean = false;
                break;
            }
        }
        if all_clean {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("task 999 must not be created on any node");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_plane_apply_op_routes_to_correct_sm() {
    use bee_control::kv::TxnError;
    let mut cp = ControlPlaneStateMachine::new();
    cp.apply_op(&Op::RegisterJob {
        job_id: 1,
        dag_hash: "x".to_string(),
        owner_node: 1, tenant: 0, })
    .unwrap();
    let err = cp
        .apply_op(&Op::Put {
            key: "k".to_string(),
            value: b"v".to_vec(),
        })
        .expect_err("Put must be rejected on CP SM");
    assert_eq!(err, TxnError::WrongSm);
}
