use bee_control::scheduler::{
    FirstFitDecreasingScheduler, NodeCapacity, Scheduler, TaskPlacement, TaskRequirement,
};

fn mk_req(id: u32, cpu: u32, mem: u32) -> TaskRequirement {
    TaskRequirement { task_id: id, cpu_millicores: cpu, mem_mb: mem }
}

fn mk_node(id: u32, cpu: u32, mem: u32) -> NodeCapacity {
    NodeCapacity { node_id: id, cpu_millicores_total: cpu, mem_mb_total: mem }
}

#[test]
fn ffd_packs_3_tasks_of_500m_onto_3_nodes_of_1000m_splitting_2_and_1() {
    let s = FirstFitDecreasingScheduler::new();
    let tasks = vec![mk_req(1, 500, 0), mk_req(2, 500, 0), mk_req(3, 500, 0)];
    let nodes = vec![mk_node(1, 1000, 1024), mk_node(2, 1000, 1024), mk_node(3, 1000, 1024)];
    let placements = s.place(&tasks, &nodes);
    let per_node: std::collections::HashMap<u32, Vec<u32>> =
        placements
            .iter()
            .zip(tasks.iter())
            .filter_map(|(p, t)| p.as_ref().map(|p| (p.node_id, t.task_id)))
            .fold(std::collections::HashMap::new(), |mut acc, (n, t)| {
                acc.entry(n).or_default().push(t);
                acc
            });
    let node1_tasks = per_node.get(&1).map(|v| v.len()).unwrap_or(0);
    let node2_tasks = per_node.get(&2).map(|v| v.len()).unwrap_or(0);
    let node3_tasks = per_node.get(&3).map(|v| v.len()).unwrap_or(0);
    assert_eq!(node1_tasks, 2, "node 1 must host 2 tasks (500+500=1000)");
    assert_eq!(node2_tasks, 1, "node 2 must host 1 task (the third 500m)");
    assert_eq!(node3_tasks, 0, "node 3 must be unused");
}

#[test]
fn ffd_does_not_overcommit_a_node() {
    let s = FirstFitDecreasingScheduler::new();
    let tasks = vec![
        mk_req(1, 400, 0),
        mk_req(2, 400, 0),
        mk_req(3, 400, 0),
        mk_req(4, 400, 0),
        mk_req(5, 400, 0),
    ];
    let nodes = vec![
        mk_node(1, 1000, 1024),
        mk_node(2, 1000, 1024),
        mk_node(3, 1000, 1024),
    ];
    let placements = s.place(&tasks, &nodes);

    let mut per_node_load: std::collections::HashMap<u32, u32> =
        std::collections::HashMap::new();
    for (slot, task) in placements.iter().zip(tasks.iter()) {
        let p = slot.as_ref().expect("FFD must place every 400m task on 1000m nodes");
        *per_node_load.entry(p.node_id).or_insert(0) += task.cpu_millicores;
    }
    for (&node_id, &load) in &per_node_load {
        assert!(load <= 1000, "node {node_id} over-committed: {load} mCPU");
    }
    assert_eq!(placements.iter().filter(|p| p.is_some()).count(), 5);
}

#[test]
fn ffd_returns_none_when_a_task_does_not_fit_any_node() {
    let s = FirstFitDecreasingScheduler::new();
    let tasks = vec![mk_req(1, 500, 0), mk_req(2, 2000, 0)];
    let nodes = vec![mk_node(1, 1000, 1024)];
    let placements = s.place(&tasks, &nodes);
    assert!(placements[0].is_some(), "500m fits on 1000m");
    assert!(placements[1].is_none(), "2000m does not fit on 1000m");
}

#[test]
fn ffd_considers_memory_in_addition_to_cpu() {
    let s = FirstFitDecreasingScheduler::new();
    // Task needs 800 MB; node 1 only has 512 MB.
    let tasks = vec![mk_req(1, 100, 800)];
    let nodes = vec![
        mk_node(1, 1000, 512),
        mk_node(2, 1000, 1024),
    ];
    let placements = s.place(&tasks, &nodes);
    assert_eq!(placements[0].as_ref().unwrap().node_id, 2);
}

#[test]
fn ffd_returns_all_none_for_empty_node_set() {
    let s = FirstFitDecreasingScheduler::new();
    let tasks = vec![mk_req(1, 100, 0)];
    let nodes: Vec<NodeCapacity> = vec![];
    let placements = s.place(&tasks, &nodes);
    assert!(placements.iter().all(|p| p.is_none()));
}

#[test]
fn scheduler_is_pluggable_via_trait_object() {
    // Custom scheduler: all tasks pinned to node 0.
    struct PinToZero;
    impl Scheduler for PinToZero {
        fn place(
            &self,
            tasks: &[TaskRequirement],
            nodes: &[NodeCapacity],
        ) -> Vec<Option<TaskPlacement>> {
            let Some(first) = nodes.first() else {
                return (0..tasks.len()).map(|_| None).collect();
            };
            let pinned = first.node_id;
            tasks
                .iter()
                .map(|t| Some(TaskPlacement { task_id: t.task_id, node_id: pinned }))
                .collect()
        }
    }
    let s: Box<dyn Scheduler> = Box::new(PinToZero);
    let tasks = vec![mk_req(1, 100, 0), mk_req(2, 100, 0), mk_req(3, 100, 0)];
    let nodes = vec![mk_node(7, 1000, 1024), mk_node(8, 1000, 1024)];
    let placements = s.place(&tasks, &nodes);
    for p in &placements {
        let p = p.as_ref().unwrap();
        assert_eq!(p.node_id, 7, "all tasks pinned to first node");
    }
}
