//! S24: per-Task metrics in the ControlPlane + display.

use bee_control::control_plane::ControlPlaneStateMachine;
use bee_control::kv::{Op, TaskMetricsSnapshot, TaskStatus};

#[test]
fn task_metrics_round_trip_via_op() {
    let mut cp = ControlPlaneStateMachine::new();

    // Register a Job + Task.
    cp.apply_op(&Op::RegisterJob {
        job_id: 1,
        dag_hash: "x".into(),
        owner_node: 1,
        tenant: 0,
        dependencies: vec![],
    })
    .unwrap();
    cp.apply_op(&Op::RegisterTask {
        task_id: 1,
        job_id: 1,
        phase_id: 0,
        owner_node: 1,
        status: TaskStatus::Pending,
        started_at_ms: 0,
    })
    .unwrap();

    // Worker pushes a snapshot.
    let snap = TaskMetricsSnapshot {
        events_processed_total: 42,
        latency_bucket_counts: [5, 10, 20, 5, 2],
        backpressure_wait_seconds_total_ns: 1_500_000_000, // 1.5s
    };
    cp.apply_op(&Op::RecordTaskMetrics {
        task_id: 1,
        snapshot: snap.clone(),
    })
    .unwrap();

    // View reads the metrics.
    let stored = cp.get_task_metrics(1).expect("metrics present");
    assert_eq!(stored.events_processed_total, 42);
    assert_eq!(stored.latency_bucket_counts, [5, 10, 20, 5, 2]);
    assert_eq!(stored.backpressure_wait_seconds_total_ns, 1_500_000_000);
}

#[tokio::test(flavor = "current_thread")]
async fn format_diagnostics_renders_real_metrics() {
    use bee_control::diagnostics_view::format_task_diagnostics;

    let mut cp = ControlPlaneStateMachine::new();
    cp.apply_op(&Op::RegisterJob {
        job_id: 1,
        dag_hash: "x".into(),
        owner_node: 1,
        tenant: 0,
        dependencies: vec![],
    })
    .unwrap();
    cp.apply_op(&Op::RegisterTask {
        task_id: 1,
        job_id: 1,
        phase_id: 0,
        owner_node: 1,
        status: TaskStatus::Running,
        started_at_ms: 0,
    })
    .unwrap();
    cp.apply_op(&Op::RecordTaskMetrics {
        task_id: 1,
        snapshot: TaskMetricsSnapshot {
            events_processed_total: 5,
            latency_bucket_counts: [1, 2, 2, 0, 0],
            backpressure_wait_seconds_total_ns: 0,
        },
    })
    .unwrap();
    let s = format_task_diagnostics(&cp, 1).await.unwrap();
    assert!(
        s.contains("events_processed_total: 5"),
        "missing events line:\n{s}"
    );
    assert!(
        s.contains("1 / 2 / 2 / 0 / 0"),
        "missing bucket counts:\n{s}"
    );
}
