//! `bee-kv-test` — S06 smoke test binary.
//!
//! Spins up a single-node Raft loop (`RaftNode`) backed by the KV state
//! machine, runs 100 put/cas round-trips, then verifies the state machine
//! contains the expected values. Prints `ok` on success. Exits with
//! non-zero status on the first mismatch.

use bee_control::{Op, RaftNode, TxnError};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut node = RaftNode::new();

    for i in 0..100u32 {
        let key = format!("state/task/smoke/key-{i:03}");
        let value = format!("value-{i:03}").into_bytes();
        node.apply(Op::Put { key: key.clone(), value: value.clone() })?;
        node.apply(Op::Cas {
            key,
            expected: Some(value.clone()),
            new: value,
        })?;
    }

    for i in 0..100u32 {
        let key = format!("state/task/smoke/key-{i:03}");
        let expected = format!("value-{i:03}").into_bytes();
        let got = node.state_machine().get(&key);
        if got.as_deref() != Some(expected.as_slice()) {
            eprintln!("mismatch at {key}: got {got:?}, expected {expected:?}");
            std::process::exit(1);
        }
    }

    let _ = TxnError::NestedTxn;
    println!("ok");
    Ok(())
}
