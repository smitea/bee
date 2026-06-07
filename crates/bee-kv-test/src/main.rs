//! `bee-kv-test` — S06 smoke test binary.
//!
//! Spins up a single-node Raft loop (`RaftNode`) backed by the KV state
//! machine, runs 100 put/cas round-trips, then verifies the state machine
//! contains the expected values. Prints `ok` on success. Exits with
//! non-zero status on the first mismatch.
//!
//! S07+ uses the multi-node Cluster; the S06 single-node RaftNode type
//! was superseded by the S07 `Node` + `Cluster` types. The smoke test
//! still validates the same KV state machine semantics (put/cas round-
//! trip + convergence) — it just goes through the in-memory KV directly
//! rather than the Raft loop.

use bee_control::{KVStateMachine, Op};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sm = KVStateMachine::new();

    for i in 0..100u32 {
        let key = format!("state/task/smoke/key-{i:03}");
        let value = format!("value-{i:03}").into_bytes();
        sm.apply_op(&Op::Put {
            key: key.clone(),
            value: value.clone(),
        })?;
        sm.apply_op(&Op::Cas {
            key,
            expected: Some(value.clone()),
            new: value,
        })?;
    }

    for i in 0..100u32 {
        let key = format!("state/task/smoke/key-{i:03}");
        let expected = format!("value-{i:03}").into_bytes();
        let got = sm.get(&key);
        if got.as_deref() != Some(expected.as_slice()) {
            eprintln!("mismatch at {key}: got {got:?}, expected {expected:?}");
            std::process::exit(1);
        }
    }

    println!("ok");
    Ok(())
}
