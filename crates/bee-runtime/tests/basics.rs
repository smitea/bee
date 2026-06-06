use bee_runtime::{AdapterRef, Dag, Handler, PassthroughHandler, Phase, RuntimeError};

#[tokio::test]
async fn one_phase_passthrough_returns_input_unchanged() {
    let mut phase = Phase::new(0, "p1", PassthroughHandler);
    let input: Vec<u8> = vec![1, 2, 3, 4, 5];
    let output = phase.run(input.clone()).await.expect("passthrough should succeed");
    assert_eq!(output, input);
}

#[test]
fn dag_vertices_returns_added_phases() {
    let mut dag = Dag::<PassthroughHandler>::new();
    dag.add_phase(Phase::new(0, "a", PassthroughHandler));
    dag.add_phase(Phase::new(1, "b", PassthroughHandler));
    assert_eq!(dag.vertices().len(), 2);
    assert_eq!(dag.vertices()[0].id, 0);
    assert_eq!(dag.vertices()[0].name, "a");
    assert_eq!(dag.vertices()[1].id, 1);
    assert_eq!(dag.vertices()[1].name, "b");
}

#[test]
fn dag_edges_returns_added_edges_in_order() {
    let mut dag = Dag::<PassthroughHandler>::new();
    dag.add_phase(Phase::new(0, "a", PassthroughHandler));
    dag.add_phase(Phase::new(1, "b", PassthroughHandler));
    dag.add_edge(0, 1);
    dag.add_edge(1, 1);
    assert_eq!(dag.edges(), &[(0, 1), (1, 1)]);
}

#[tokio::test]
async fn phase_finish_lifecycle_succeeds_by_default() {
    let phase = Phase::new(0, "p1", PassthroughHandler);
    phase.finish().await.expect("finish must succeed");
}

#[test]
fn phase_with_adapter_carries_adapter_reference() {
    let phase = Phase::new(7, "datasource-phase", PassthroughHandler)
        .with_adapter(AdapterRef(42));
    assert_eq!(phase.adapter, Some(AdapterRef(42)));
    assert_eq!(phase.id, 7);
    assert_eq!(phase.name, "datasource-phase");
}

#[tokio::test]
async fn handler_can_return_typed_output_via_associated_types() {
    struct DoubleHandler;
    impl Handler for DoubleHandler {
        type Input = i64;
        type Output = i64;
        async fn handle(&mut self, input: i64) -> Result<i64, RuntimeError> {
            Ok(input * 2)
        }
        async fn finish(self) -> Result<(), RuntimeError> {
            Ok(())
        }
    }
    let mut phase = Phase::new(0, "double", DoubleHandler);
    let out = phase.run(21).await.unwrap();
    assert_eq!(out, 42);
}
