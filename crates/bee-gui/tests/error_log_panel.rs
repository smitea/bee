//! Integration test stub: error_log_panel.
//!
//! Full assertion requires the cluster harness + a deliberately-invalid
//! RPC that the server replies `AdminResponse::Error` to; deferred to
//! S-1c. The GUI-side LogPanel rendering is unit-tested by
//! `crate::log_panel::tests`.

#[test]
#[ignore = "requires cluster harness + invalid-RPC helper (S-1c)"]
fn rpc_server_error_appears_in_log_panel() {}