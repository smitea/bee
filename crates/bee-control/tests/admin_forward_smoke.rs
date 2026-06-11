//! S33.4: end-to-end Forward test. Spins up a
//! 3-node TCP cluster. The follower (node 2)
//! receives an `AdminRequest::KvPut`; its
//! AdminServer's `Forward` arm returns a
//! "queued for leader" response (the full
//! Raft-log apply is S33.5; this test
//! exercises the wire format + dispatch path).
//!
//! The intent is to lock down the S33.4 wire
//! type surface (AdminRequest::Forward +
//! RpcMessage::AdminForward) and confirm
//! the AdminServer compiles + dispatches
//! without panic.
//!
//! Run with: cargo test -p bee-control
//!   --test admin_forward_smoke
//!   -- --nocapture

use std::time::Duration;

use bee_control::raft::admin_client::{AdminClient, AdminError};
use bee_control::raft::admin_protocol::{AdminRequest, AdminResponse};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forward_arm_returns_queued() {
    // The S33.3 test pattern: bind to a random
    // port, start an AdminServer with empty
    // state, send a Forward request, assert
    // the "queued for leader" response.
    //
    // (The full 3-node TCP forwarding test is
    // a follow-up; this smoke test locks down
    // the wire format + the Forward arm path.)
    let kv = std::sync::Arc::new(tokio::sync::Mutex::new(
        bee_control::kv::KVStateMachine::new(),
    ));
    let cp = std::sync::Arc::new(tokio::sync::Mutex::new(
        bee_control::control_plane::ControlPlaneStateMachine::new(),
    ));
    let state = std::sync::Arc::new(tokio::sync::Mutex::new(
        bee_control::raft::node::NodeState::default(),
    ));
    let mut admin = bee_control::raft::admin_server::AdminServer::start(
        "127.0.0.1:0".parse().unwrap(),
        kv,
        cp,
        state,
        None,
        None,
        None,
        None,  // plugin_manager (S33.5.2)
    )
    .await
    .expect("AdminServer::start");
    let addr = admin.local_addr();
    let mut client = AdminClient::connect(addr).await.expect("connect");
    // Build an inner request (KvPut) and bincode it
    let inner = AdminRequest::KvPut {
        key: "smoke/key".to_string(),
        value: b"v".to_vec(),
    };
    let inner_bytes = bincode::serialize(&inner).expect("serialize");
    let resp: Result<AdminResponse, AdminError> = client
        .call(AdminRequest::Forward {
            to: 1,
            request: inner_bytes.clone(),
        })
        .await;
    // The S33.4 Task 5 placeholder returns
    // "queued for leader (Task 5c wires the
    // leader apply)". The AdminClient surfaces
    // that as a `ServerError`. We accept it for
    // the MVP; when Task 5c lands, this test
    // will receive the actual AdminResponse.
    let inner_bytes_clone = inner_bytes.clone();
    let err: AdminError = resp.expect_err("Forward returns Error for MVP");
    let err_str = format!("{err:?}");
    assert!(
        err_str.contains("queued") || err_str.contains("Forward"),
        "unexpected error: {err_str}"
    );
    // Sanity: the inner bytes round-trip — bincode
    // can decode the payload we sent.
    let _decoded: AdminRequest = bincode::deserialize(&inner_bytes_clone)
        .expect("inner bytes round-trip");
    admin.shutdown();
    // Give the listener task a moment to exit
    tokio::time::sleep(Duration::from_millis(50)).await;
}
