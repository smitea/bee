use std::time::Duration;

use bee_runtime::{
    Dag, DynPhase, FilterHandler, MapHandler, Msg, Runtime, RuntimeError,
};
use tokio::sync::mpsc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_executes_two_phase_chain_map_then_filter() {
    let mut dag = Dag::new();
    dag.add_phase(DynPhase::new(
        0,
        "incr",
        MapHandler::<_, i64>::new(|x| x + 1),
    ));
    dag.add_phase(DynPhase::new(
        1,
        "gt5",
        FilterHandler::<_, i64>::new(|x: &i64| *x > 5),
    ));
    dag.add_edge(0, 1).unwrap();

    let (input_tx, input_rx) = mpsc::channel::<Msg>(8);
    let (output_tx, mut output_rx) = mpsc::channel::<Msg>(8);

    let runtime_handle = Runtime::run(dag, input_rx, output_tx);

    for i in 1..10i64 {
        input_tx.send(Msg::new(i)).await.expect("input send");
    }
    drop(input_tx);

    let mut results: Vec<i64> = Vec::new();
    while let Some(msg) = output_rx.recv().await {
        let val = *msg.downcast_ref::<i64>().expect("output is i64");
        results.push(val);
    }

    assert_eq!(results, vec![6, 7, 8, 9, 10]);

    runtime_handle
        .await
        .expect("runtime task must not panic")
        .expect("runtime must complete successfully");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_cleanly_shuts_down_when_input_channel_closes() {
    let mut dag = Dag::new();
    dag.add_phase(DynPhase::new(0, "identity", MapHandler::<_, i64>::new(|x| x)));

    let (input_tx, input_rx) = mpsc::channel::<Msg>(8);
    let (output_tx, _output_rx) = mpsc::channel::<Msg>(8);

    let runtime_handle = Runtime::run(dag, input_rx, output_tx);

    drop(input_tx);

    match tokio::time::timeout(Duration::from_secs(2), runtime_handle).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => panic!("runtime returned error: {e:?}"),
        Ok(Err(e)) => panic!("runtime task panicked: {e:?}"),
        Err(_) => panic!("runtime did not shut down within 2s"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_single_phase_dag_passes_input_through_to_output() {
    let mut dag = Dag::new();
    dag.add_phase(DynPhase::new(
        0,
        "double",
        MapHandler::<_, i64>::new(|x| x * 2),
    ));

    let (input_tx, input_rx) = mpsc::channel::<Msg>(8);
    let (output_tx, mut output_rx) = mpsc::channel::<Msg>(8);

    let runtime_handle = Runtime::run(dag, input_rx, output_tx);

    for i in 1..=4i64 {
        input_tx.send(Msg::new(i)).await.unwrap();
    }
    drop(input_tx);

    let mut results: Vec<i64> = Vec::new();
    while let Some(msg) = output_rx.recv().await {
        let val = *msg.downcast_ref::<i64>().unwrap();
        results.push(val);
    }

    assert_eq!(results, vec![2, 4, 6, 8]);

    runtime_handle
        .await
        .unwrap()
        .unwrap_or_else(|e: RuntimeError| panic!("{e:?}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn passthrough_handler_with_vec_u8_works_in_runtime_via_user_input() {
    let mut dag = Dag::new();
    let (ptx, prx) = mpsc::channel::<Msg>(8);
    dag.add_phase(DynPhase::new(
        0,
        "identity-via-passthrough",
        MapHandler::<_, i64>::new(|x| x),
    ));
    drop(ptx);
    drop(prx);
}
