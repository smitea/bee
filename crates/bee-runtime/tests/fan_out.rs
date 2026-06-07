use bee_runtime::{Dag, DynPhase, MapHandler, Msg, Runtime, RuntimeError};
use tokio::sync::mpsc;

#[test]
fn dag_rejects_self_loop_edge() {
    let mut dag = Dag::new();
    dag.add_phase(DynPhase::new(0, "a", MapHandler::<_, i64>::new(|x| x)));
    assert!(
        matches!(dag.add_edge(0, 0), Err(RuntimeError::Topology(_))),
        "self-loop must be rejected"
    );
}

#[test]
fn dag_rejects_edge_that_would_close_a_cycle() {
    let mut dag = Dag::new();
    dag.add_phase(DynPhase::new(0, "a", MapHandler::<_, i64>::new(|x| x)));
    dag.add_phase(DynPhase::new(1, "b", MapHandler::<_, i64>::new(|x| x)));
    dag.add_phase(DynPhase::new(2, "c", MapHandler::<_, i64>::new(|x| x)));
    dag.add_edge(0, 1).unwrap();
    dag.add_edge(1, 2).unwrap();
    assert!(
        matches!(dag.add_edge(2, 0), Err(RuntimeError::Topology(_))),
        "back-edge closing the cycle must be rejected"
    );
}

#[test]
fn dag_rejects_cycle_via_intermediate() {
    let mut dag = Dag::new();
    dag.add_phase(DynPhase::new(0, "a", MapHandler::<_, i64>::new(|x| x)));
    dag.add_phase(DynPhase::new(1, "b", MapHandler::<_, i64>::new(|x| x)));
    dag.add_phase(DynPhase::new(2, "c", MapHandler::<_, i64>::new(|x| x)));
    dag.add_phase(DynPhase::new(3, "d", MapHandler::<_, i64>::new(|x| x)));
    dag.add_edge(0, 1).unwrap();
    dag.add_edge(1, 2).unwrap();
    dag.add_edge(2, 3).unwrap();
    assert!(
        matches!(dag.add_edge(3, 1), Err(RuntimeError::Topology(_))),
        "back-edge into middle of chain must be rejected"
    );
}

#[test]
fn dag_accepts_diamond_topology() {
    let mut dag = Dag::new();
    dag.add_phase(DynPhase::new(0, "src", MapHandler::<_, i64>::new(|x| x)));
    dag.add_phase(DynPhase::new(1, "a", MapHandler::<_, i64>::new(|x| x + 100)));
    dag.add_phase(DynPhase::new(2, "b", MapHandler::<_, i64>::new(|x| x + 200)));
    dag.add_phase(DynPhase::new(3, "sink", MapHandler::<_, i64>::new(|x| x)));
    dag.add_edge(0, 1).unwrap();
    dag.add_edge(0, 2).unwrap();
    dag.add_edge(1, 3).unwrap();
    dag.add_edge(2, 3).unwrap();
    assert_eq!(dag.vertices().len(), 4);
    assert_eq!(dag.edges().len(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_executes_diamond_fan_out_fan_in() {
    let mut dag = Dag::new();
    dag.add_phase(DynPhase::new(0, "src", MapHandler::<_, i64>::new(|x| x)));
    dag.add_phase(DynPhase::new(
        1,
        "branch_a",
        MapHandler::<_, i64>::new(|x| x + 100),
    ));
    dag.add_phase(DynPhase::new(
        2,
        "branch_b",
        MapHandler::<_, i64>::new(|x| x + 200),
    ));
    dag.add_phase(DynPhase::new(3, "sink", MapHandler::<_, i64>::new(|x| x)));
    dag.add_edge(0, 1).unwrap();
    dag.add_edge(0, 2).unwrap();
    dag.add_edge(1, 3).unwrap();
    dag.add_edge(2, 3).unwrap();

    let (input_tx, input_rx) = mpsc::channel::<Msg>(8);
    let (output_tx, mut output_rx) = mpsc::channel::<Msg>(32);

    let runtime_handle = Runtime::run(dag, input_rx, output_tx);

    for i in 1..=3i64 {
        input_tx.send(Msg::new(i)).await.unwrap();
    }
    drop(input_tx);

    let mut results: Vec<i64> = Vec::new();
    while let Some(msg) = output_rx.recv().await {
        let val = *msg.downcast_ref::<i64>().expect("output is i64");
        results.push(val);
    }

    results.sort();
    assert_eq!(results, vec![101, 102, 103, 201, 202, 203]);

    runtime_handle
        .await
        .expect("runtime task must not panic")
        .expect("runtime must complete successfully");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_fan_out_two_branches_run_in_parallel() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let concurrent = Arc::new(AtomicUsize::new(0));
    let max_concurrent = Arc::new(AtomicUsize::new(0));

    let c1 = concurrent.clone();
    let m1 = max_concurrent.clone();
    let c2 = concurrent.clone();
    let m2 = max_concurrent.clone();

    struct CountingHandler {
        active: Arc<AtomicUsize>,
        max: Arc<AtomicUsize>,
    }
    impl Handler for CountingHandler {
        type Input = i64;
        type Output = i64;
        fn handle(
            &mut self,
            input: i64,
        ) -> impl std::future::Future<Output = Result<Option<i64>, RuntimeError>> + Send {
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            let prev_max = self.max.load(Ordering::SeqCst);
            if now > prev_max {
                self.max.store(now, Ordering::SeqCst);
            }
            let active = self.active.clone();
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(Some(input))
            }
        }
        fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send {
            async move { Ok(()) }
        }
    }

    use bee_runtime::Handler;

    let mut dag = Dag::new();
    dag.add_phase(DynPhase::new(0, "src", MapHandler::<_, i64>::new(|x| x)));
    dag.add_phase(DynPhase::new(
        1,
        "branch_a",
        CountingHandler { active: c1, max: m1 },
    ));
    dag.add_phase(DynPhase::new(
        2,
        "branch_b",
        CountingHandler { active: c2, max: m2 },
    ));
    dag.add_phase(DynPhase::new(3, "sink", MapHandler::<_, i64>::new(|x| x)));
    dag.add_edge(0, 1).unwrap();
    dag.add_edge(0, 2).unwrap();
    dag.add_edge(1, 3).unwrap();
    dag.add_edge(2, 3).unwrap();

    let (input_tx, input_rx) = mpsc::channel::<Msg>(8);
    let (output_tx, mut output_rx) = mpsc::channel::<Msg>(32);

    let runtime_handle = Runtime::run(dag, input_rx, output_tx);

    for i in 1..=4i64 {
        input_tx.send(Msg::new(i)).await.unwrap();
    }
    drop(input_tx);

    let mut count = 0;
    while output_rx.recv().await.is_some() {
        count += 1;
    }
    assert_eq!(count, 8);

    let observed_max = max_concurrent.load(Ordering::SeqCst);
    assert!(
        observed_max >= 2,
        "branches A and B should run concurrently, observed max active = {observed_max}"
    );

    runtime_handle.await.unwrap().unwrap();
}
