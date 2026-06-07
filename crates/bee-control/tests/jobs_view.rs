//! S27: `bee jobs` + `bee jobs inspect <JobId>` end-to-end.
//!
//! These tests deploy a Job + Tasks via the Cluster submit path,
//! then format the ControlPlane state using `jobs_view` and assert
//! the table + ASCII DAG + color codes render correctly.

use std::time::Duration;

use bee_control::jobs_view::{format_job_inspect, format_jobs};
use bee_control::kv::{JobLifecycleState, Op, TaskStatus};
use bee_control::raft::cluster::{Cluster, ClusterConfig};

async fn fresh_cluster() -> Cluster {
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader");
    cluster
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bee_jobs_on_fresh_cluster_is_empty() {
    // S27 acceptance: `bee jobs` works on a fresh cluster (returns empty).
    let cluster = fresh_cluster().await;
    let (_, handle) = cluster.nodes().next().unwrap();
    let cp = handle.cp.lock().await;
    let output = format_jobs(&cp);
    assert!(output.contains("(no jobs)"), "fresh cluster must show empty list, got:\n{output}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bee_jobs_after_deploy_shows_the_job() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 1,
                dag_hash: "linear-3".into(),
                owner_node: leader,
            },
        )
        .await
        .unwrap();
    cluster
        .submit(
            leader,
            Op::RegisterTask {
                task_id: 1,
                job_id: 1,
                phase_id: 0,
                owner_node: 1,
                status: TaskStatus::Scheduled,
                started_at_ms: 0,
            },
        )
        .await
        .unwrap();
    cluster
        .submit(
            leader,
            Op::RegisterTask {
                task_id: 2,
                job_id: 1,
                phase_id: 0,
                owner_node: 2,
                status: TaskStatus::Scheduled,
                started_at_ms: 0,
            },
        )
        .await
        .unwrap();
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
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (_, handle) = cluster.nodes().next().unwrap();
    let cp = handle.cp.lock().await;
    let output = format_jobs(&cp);
    assert!(output.contains("linear-3"), "job name missing:\n{output}");
    assert!(output.contains("|     2 |"), "task count = 2 missing:\n{output}");
    // Color code: running → green
    assert!(output.contains("\x1b[32mrunning\x1b[0m"), "missing green color:\n{output}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bee_jobs_inspect_shows_dag_and_per_task_status() {
    // S27 acceptance: `bee jobs inspect <JobId>` shows a DAG
    // diagram and per-Task status.
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 1,
                dag_hash: "my-pipeline".into(),
                owner_node: leader,
            },
        )
        .await
        .unwrap();
    for tid in 1..=3 {
        cluster
            .submit(
                leader,
                Op::RegisterTask {
                    task_id: tid,
                    job_id: 1,
                    phase_id: 0,
                    owner_node: tid,
                    status: TaskStatus::Running,
                    started_at_ms: 0,
                },
            )
            .await
            .unwrap();
    }
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
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (_, handle) = cluster.nodes().next().unwrap();
    let cp = handle.cp.lock().await;
    let output = format_job_inspect(&cp, 1).expect("inspect");

    assert!(output.contains("Job 1"), "missing header:\n{output}");
    assert!(output.contains("my-pipeline"));
    assert!(output.contains("tasks (3)"));
    assert!(output.contains("Task   1"));
    assert!(output.contains("Task   2"));
    assert!(output.contains("Task   3"));
    assert!(output.contains("DAG:"));
    // DAG connectors
    assert!(output.contains("├─") || output.contains("└─"), "missing DAG connector:\n{output}");
    // Per-Task color: all running → green
    let green_count = output.matches("\x1b[32mrunning\x1b[0m").count();
    assert!(green_count >= 3, "expected >= 3 green 'running' cells, got {green_count}:\n{output}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bee_jobs_inspect_unknown_job_returns_none_at_library_level() {
    let cluster = fresh_cluster().await;
    let (_, handle) = cluster.nodes().next().unwrap();
    let cp = handle.cp.lock().await;
    assert!(format_job_inspect(&cp, 999).is_none());
}

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bee_jobs_color_codes_for_different_lifecycles_s27_acceptance() {
    // Color codes per S27 acceptance:
    // green = running, yellow = waiting/migrating, red = failed
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    // Three jobs in different lifecycles
    for (id, state) in [
        (1u32, JobLifecycleState::Running),
        (2, JobLifecycleState::WaitingForUpstream),
        (3, JobLifecycleState::Failed),
    ] {
        cluster
            .submit(
                leader,
                Op::RegisterJob {
                    job_id: id,
                    dag_hash: format!("job-{id}"),
                    owner_node: leader,
                },
            )
            .await
            .unwrap();
        cluster
            .submit(
                leader,
                Op::UpdateJobLifecycle { job_id: id, state },
            )
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (_, handle) = cluster.nodes().next().unwrap();
    let cp = handle.cp.lock().await;
    let output = format_jobs(&cp);
    assert!(output.contains("\x1b[32mrunning\x1b[0m"), "missing green:\n{output}");
    assert!(output.contains("\x1b[33mwaiting_for_upstream\x1b[0m"), "missing yellow:\n{output}");
    assert!(output.contains("\x1b[31mfailed\x1b[0m"), "missing red:\n{output}");
}
