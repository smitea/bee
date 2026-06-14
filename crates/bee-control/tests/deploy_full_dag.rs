//! S33.5.3 Task 5: end-to-end test for the
//! `Deploy` arm. Sends a 2-SELECT SQL,
//! asserts the response has job_id=1 +
//! 2 task_ids, then verifies the control
//! plane has 1 Job + 2 Tasks.

use std::sync::Arc;

use bee_control::control_plane::ControlPlaneStateMachine;
use bee_control::kv::KVStateMachine;
use bee_control::raft::admin_client::AdminClient;
use bee_control::raft::admin_protocol::{
    AdminRequest, AdminResponse,
};
use bee_control::raft::admin_server::AdminServer;
use bee_control::raft::node::NodeState;
use tokio::sync::Mutex;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_two_phase_sql_creates_job_and_two_tasks() {
    let kv = Arc::new(Mutex::new(KVStateMachine::new()));
    let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new()));
    let state = Arc::new(Mutex::new(NodeState::default()));
    let mut admin = AdminServer::start(
        "127.0.0.1:0".parse().unwrap(),
        kv,
        cp.clone(),
        state,
        None,
        None,
        None,
        None, // S33.5.2 plugin_manager = None
    )
    .await
    .expect("AdminServer::start");
    let mut client = AdminClient::connect(admin.local_addr())
        .await
        .expect("connect");
    let resp = client
        .call(AdminRequest::Deploy {
            sql_text: "SELECT * FROM binance.subscribe('BTC/USDT', '5min'); \
                       SELECT avg(price) FROM ticks;"
                .to_string(),
            owner_node: 1,
        })
        .await
        .expect("call");
    let (job_id, task_ids) = match resp {
        AdminResponse::DeployAck {
            job_id,
            task_ids,
            error_msg,
        } => {
            assert!(
                error_msg.is_empty(),
                "deploy failed: {error_msg}"
            );
            (job_id, task_ids)
        }
        other => panic!("expected DeployAck, got: {other:?}"),
    };
    assert_eq!(job_id, 1);
    assert_eq!(task_ids.len(), 2);
    assert_eq!(task_ids[0], 1);
    assert_eq!(task_ids[1], 2);
    // Verify the control plane has 1 Job +
    // 2 Tasks.
    let cp_locked = cp.lock().await;
    let jobs = cp_locked.list_jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, 1);
    let tasks = cp_locked.list_tasks();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].job_id, 1);
    assert_eq!(tasks[0].phase_id, 1);
    assert_eq!(tasks[1].job_id, 1);
    assert_eq!(tasks[1].phase_id, 2);
    admin.shutdown();
}
