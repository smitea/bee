//! Integration test stub: connection_smoke.
//!
//! Full assertion requires a real in-process 3-node Cluster harness
//! (S-1c follow-up: extract `bee-control::test_utils::cluster`). The
//! current build verifies that the GUI-side types compile + the unit
//! tests in `src/connection.rs` cover the state machine + tagging paths.
//!
//! Marking `#[ignore]` so the test runner doesn't fail without the
//! cluster harness; flip to a real integration test once the harness
//! ships.

#[test]
#[ignore = "requires extracted in-process 3-node cluster test-utils (S-1c)"]
fn connect_and_ping_succeeds() {
    // See crates/bee-control/tests/raft_cluster.rs for the cluster harness
    // pattern; replicate the bind + connect dance once it is moved into
    // a public test-utils module.
}