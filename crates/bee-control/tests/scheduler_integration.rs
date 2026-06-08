use std::collections::HashMap;

use bee_control::deployer::{Deployer, DeployerConfig, HandlerKind, Pipeline, TaskSpec};
use bee_control::scheduler::{NodeCapacity, Scheduler, TaskPlacement, TaskRequirement};
use bee_control::Edge;

fn linear_pipeline(tasks: Vec<TaskSpec>, edges: Vec<Edge>) -> Pipeline {
    Pipeline {
        name: "sched-test".to_string(),
        tasks,
        edges,
        stream_identities: vec![],
    }
}

fn started_spec(id: u32, cpu: u32, mem: u32) -> TaskSpec {
    TaskSpec {
        task_id: id,
        phase_id: 0,
        handler_kind: HandlerKind::Started { tag: format!("T{id}") },
        cpu_millicores: cpu,
        mem_mb: mem,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deployer_uses_scheduler_instead_of_round_robin() {
    // Custom scheduler: all tasks go to worker 1.
    struct AllToWorkerOne;
    impl Scheduler for AllToWorkerOne {
        fn place(
            &self,
            tasks: &[TaskRequirement],
            nodes: &[NodeCapacity],
        ) -> Vec<Option<TaskPlacement>> {
            let pinned = nodes[0].node_id;
            tasks
                .iter()
                .map(|t| Some(TaskPlacement { task_id: t.task_id, node_id: pinned }))
                .collect()
        }
    }
    let config = DeployerConfig::default();
    let mut deployer = Deployer::with_scheduler(config, Box::new(AllToWorkerOne)).await;

    let tasks = vec![started_spec(1, 0, 0), started_spec(2, 0, 0), started_spec(3, 0, 0)];
    let job_id = deployer
        .deploy(linear_pipeline(tasks.clone(), vec![]))
        .await
        .unwrap();

    let mapping = deployer.job_to_node_mapping(job_id).await;
    let mut per_node_count: HashMap<u32, u32> = HashMap::new();
    for &node_id in mapping.values() {
        *per_node_count.entry(node_id).or_insert(0) += 1;
    }
    assert_eq!(per_node_count.len(), 1, "all tasks must land on the same node");
    assert_eq!(per_node_count[&1], 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deployer_default_uses_ffd_and_respects_capacity() {
    // 5 tasks of 400m, 3 workers of 1000m each.
    // FFD placement: 2+2+1 across workers.
    let config = DeployerConfig::default();
    let mut deployer = Deployer::new(config).await;

    let tasks: Vec<TaskSpec> = (1..=5u32).map(|id| started_spec(id, 400, 0)).collect();
    let job_id = deployer
        .deploy(linear_pipeline(tasks, vec![]))
        .await
        .expect("FFD must place 5×400m on 3×1000m workers");

    let mapping = deployer.job_to_node_mapping(job_id).await;
    let mut per_node_count: HashMap<u32, u32> = HashMap::new();
    for (&_task_id, &node_id) in &mapping {
        *per_node_count.entry(node_id).or_insert(0) += 1;
        let count = per_node_count[&node_id];
        assert!(count * 400 <= 1000, "node {node_id} over-committed");
    }
    let max_on_one = per_node_count.values().copied().max().unwrap_or(0);
    assert!(max_on_one <= 2, "FFD should fit 5×400m as 2+2+1; max per node = {max_on_one}");
    assert_eq!(per_node_count.values().sum::<u32>(), 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deployer_returns_insufficient_capacity_when_scheduler_cannot_place() {
    // Try to deploy 2 tasks of 1000m each on 1 worker of 1000m.
    let config = DeployerConfig {
        num_workers: 1,
        ..Default::default()
    };
    let mut deployer = Deployer::new(config).await;

    let tasks = vec![started_spec(1, 1000, 0), started_spec(2, 1000, 0)];
    let res = deployer
        .deploy(linear_pipeline(tasks, vec![]))
        .await;
    assert!(res.is_err(), "second 1000m task cannot fit on a single 1000m worker");
}
