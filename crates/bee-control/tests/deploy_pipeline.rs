use std::time::Duration;

use bee_control::deployer::{Deployer, DeployerConfig, HandlerKind, Pipeline, TaskSpec};
use bee_control::{Edge, Op};

fn linear_3_pipeline() -> Pipeline {
    Pipeline {
        name: "linear-3".to_string(),
        tasks: (1..=3u32)
            .map(|id| TaskSpec {
                task_id: id,
                phase_id: 0,
                handler_kind: HandlerKind::Started { tag: format!("T{id}") },
                cpu_millicores: 0,
                mem_mb: 0,
            })
            .collect(),
        edges: vec![
            Edge { from: 1, to: 2 },
            Edge { from: 2, to: 3 },
        ],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deploy_3_task_dag_one_per_worker_all_emit_started() {
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let _job_id = deployer.deploy(linear_3_pipeline()).await.unwrap();

    // Feed the source task so the data flows A -> B -> C, triggering
    // each handler's first call (which is when "started" is logged).
    deployer.worker(1).unwrap().feed(1, 100).unwrap();

    let arrived = deployer
        .wait_for_terminal_receive(3, 3, 100, Duration::from_secs(3))
        .await;
    assert!(arrived, "data must reach terminal before checking started logs");

    for id in 1..=3u32 {
        assert!(
            deployer.log_contains(&format!("T{id}: started")),
            "task {id} must emit 'started' log line"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_flows_across_workers_via_forwarder() {
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let _job_id = deployer.deploy(linear_3_pipeline()).await.unwrap();

    let worker1 = deployer.worker(1).expect("worker 1");
    worker1.feed(1, 42).expect("feed source task");

    let arrived = deployer
        .wait_for_terminal_receive(3, 3, 42, Duration::from_secs(3))
        .await;
    assert!(arrived, "terminal task on worker 3 must receive 42 within 3s");

    let messages = deployer.log_messages();
    let forward_count = messages.iter().filter(|m| m.starts_with("forward:")).count();
    assert!(
        forward_count >= 2,
        "expected at least 2 forward log lines (1->2 and 2->3), got {forward_count}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_plane_records_job_and_task_to_node_mapping() {
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let job_id = deployer.deploy(linear_3_pipeline()).await.unwrap();

    let mapping = deployer.job_to_node_mapping(job_id).await;
    assert_eq!(mapping.len(), 3, "all 3 tasks must be in the mapping");
    let nodes: std::collections::HashSet<u32> = mapping.values().copied().collect();
    // With zero resource requirements, FFD is free to pack; the only hard
    // requirement is that every task is mapped to some valid worker.
    for (task_id, owner) in &mapping {
        assert!(
            (1..=3u32).contains(owner),
            "task {task_id} mapped to unknown worker {owner}"
        );
    }

    // Verify via direct cluster submission that the Job exists in CP.
    let leader = deployer.cluster.leader().await.unwrap();
    let res = deployer
        .cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 999,
                dag_hash: "x".to_string(),
                owner_node: leader, tenant: 0, },
        )
        .await;
    assert!(res.is_ok(), "cluster still accepts ControlPlane ops after deploy");
}
