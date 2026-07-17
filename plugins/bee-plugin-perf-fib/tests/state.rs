use bee_plugin_perf_fib::{FibStepHandler, FibState, FibEvent};

#[tokio::test]
async fn test_state_round_trip() {
    let mut state = FibStepHandler::init_state().await.unwrap();
    let mut results = vec![];
    for n in 1..=100 {
        let (new_state, val) = FibStepHandler::handle(state, FibEvent { n }).await.unwrap();
        state = new_state;
        results.push(val);
    }
    
    // Check the first 10 values
    assert_eq!(results[0], 0);
    assert_eq!(results[1], 1);
    assert_eq!(results[2], 1);
    assert_eq!(results[3], 2);
    assert_eq!(results[4], 3);
    assert_eq!(results[5], 5);
    assert_eq!(results[6], 8);
    assert_eq!(results[7], 13);
    assert_eq!(results[8], 21);
    assert_eq!(results[9], 34);

    // Simulate restart
    let encoded_state = bincode::serialize(&state).unwrap();
    let restored_state: FibState = bincode::deserialize(&encoded_state).unwrap();
    
    let (_, val_101) = FibStepHandler::handle(restored_state, FibEvent { n: 101 }).await.unwrap();
    
    // The sequence is correctly continued.
    // results[98] is n=99, results[99] is n=100
    assert_eq!(val_101, results[98] + results[99]);
}
