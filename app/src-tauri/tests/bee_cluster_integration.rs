use std::net::SocketAddr;
use std::time::Duration;

use bee_control::raft::admin_client::AdminClient;
use bee_control::raft::admin_protocol::{
    AdminRequest, AdminResponse, JobSummary,
};
use bee_control::raft::Role;
use bee_control::test_utils::TestCluster;
use bee_control::Op;

use app_lib::connection;
use app_lib::db;
use app_lib::db::audit::{self, NewAuditEvent};

async fn boot_cluster() -> (TestCluster, SocketAddr) {
    let tc = TestCluster::boot_3_node_with_admin().await;
    let leader = tc
        .wait_for_leader(Duration::from_secs(5))
        .await
        .expect("cluster must elect a leader within 5s");
    let leader_addr = tc.admin_addrs[&leader];
    (tc, leader_addr)
}

async fn admin_call(
    addr: SocketAddr,
    req: AdminRequest,
) -> AdminResponse {
    let mut client = AdminClient::connect(addr)
        .await
        .expect("AdminClient::connect must succeed");
    client.call(req).await.expect("admin call must succeed")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cluster_topology_three_nodes_one_leader_two_followers() {
    let (tc, leader_addr) = boot_cluster().await;

    let canonical = tc.cluster.metrics().await;
    let leaders: Vec<u32> = canonical
        .iter()
        .filter(|m| m.role == Role::Leader)
        .map(|m| m.id)
        .collect();
    let followers: Vec<u32> = canonical
        .iter()
        .filter(|m| m.role == Role::Follower)
        .map(|m| m.id)
        .collect();
    assert_eq!(leaders.len(), 1, "exactly one leader expected");
    assert_eq!(followers.len(), 2, "exactly two followers expected");

    let _handle = connection::ensure_bundle(leader_addr);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if matches!(_handle.state(), connection::ConnectionState::Connected) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "connection never reached Connected; last state = {:?}",
                _handle.state()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let rx = _handle
        .call(AdminRequest::ClusterStatus)
        .await
        .expect("send ClusterStatus");
    let resp = rx.await.expect("recv").expect("ClusterStatus ok");
    let metrics = match resp {
        AdminResponse::ClusterMetrics(m) => m,
        other => panic!("expected ClusterMetrics, got {other:?}"),
    };
    assert_eq!(metrics.nodes.len(), 1, "each AdminServer reports its own node");
    assert_eq!(
        metrics.leader_id,
        Some(leaders[0]),
        "leader_id must match the in-process cluster leader"
    );
    assert!(metrics.term >= 1, "term must advance after election");

    let rx = _handle
        .call(AdminRequest::ListJobs)
        .await
        .expect("send ListJobs");
    let resp = rx.await.expect("recv").expect("ListJobs ok");
    match resp {
        AdminResponse::JobList(jobs) => {
            assert!(jobs.is_empty(), "fresh cluster has no jobs, got {jobs:?}");
        }
        other => panic!("expected JobList, got {other:?}"),
    }

    for id in 1..=3u32 {
        let addr = tc.admin_addrs[&id];
        let resp = admin_call(addr, AdminRequest::ClusterStatus).await;
        match resp {
            AdminResponse::ClusterMetrics(m) => {
                assert_eq!(m.nodes.len(), 1);
            }
            other => panic!("node {id}: expected ClusterMetrics, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn datasource_kv_round_trip_through_admin_rpc() {
    let (tc, leader_addr) = boot_cluster().await;

    let key = "ds/0/binance".to_string();
    let value = b"{\"adapter\":\"binance\"}".to_vec();
    tc.submit_kv(Op::Put {
        key: key.clone(),
        value: value.clone(),
    })
    .await
    .expect("submit_kv Put must succeed");

    let resp = admin_call(
        leader_addr,
        AdminRequest::ListKv {
            prefix: "ds/".to_string(),
        },
    )
    .await;
    let entries = match resp {
        AdminResponse::KvList(es) => es,
        other => panic!("expected KvList, got {other:?}"),
    };
    let hit = entries
        .iter()
        .find(|(k, v)| k == &key && *v == value)
        .expect("registered datasource entry must appear in ListKv");
    assert_eq!(hit.0, key);
    assert_eq!(hit.1, value);

    tc.submit_kv(Op::Del { key: key.clone() })
        .await
        .expect("submit_kv Del must succeed");

    let resp = admin_call(
        leader_addr,
        AdminRequest::ListKv {
            prefix: "ds/".to_string(),
        },
    )
    .await;
    let entries = match resp {
        AdminResponse::KvList(es) => es,
        other => panic!("expected KvList, got {other:?}"),
    };
    assert!(
        entries.iter().all(|(k, _)| k != &key),
        "deleted datasource must not appear in ListKv, got {entries:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deploy_select1_creates_job_visible_via_list_and_inspect() {
    let (tc, leader_addr) = boot_cluster().await;

    let resp = admin_call(
        leader_addr,
        AdminRequest::Deploy {
            sql_text: "SELECT 1".to_string(),
            owner_node: 0,
        },
    )
    .await;
    let (job_id, task_ids, error_msg) = match resp {
        AdminResponse::DeployAck {
            job_id,
            task_ids,
            error_msg,
        } => (job_id, task_ids, error_msg),
        other => panic!("expected DeployAck, got {other:?}"),
    };
    assert!(job_id > 0, "Deploy must assign a job_id, got {job_id}");
    assert_eq!(task_ids.len(), 1, "SELECT 1 → 1 phase → 1 task");
    assert!(
        error_msg.is_empty(),
        "Deploy must succeed, got error: {error_msg}"
    );

    let valid_owners: std::collections::HashSet<u32> =
        (0..=3u32).collect();
    let resp = admin_call(leader_addr, AdminRequest::ListJobs).await;
    let jobs: Vec<JobSummary> = match resp {
        AdminResponse::JobList(js) => js,
        other => panic!("expected JobList, got {other:?}"),
    };
    let summary = jobs
        .iter()
        .find(|j| j.job_id == job_id)
        .unwrap_or_else(|| panic!("job {job_id} must appear in ListJobs, got {jobs:?}"));
    assert_eq!(summary.task_count, 1);
    assert!(
        valid_owners.contains(&summary.owner_node),
        "owner_node {} must be a valid node id (0..=3)",
        summary.owner_node
    );
    assert!(!summary.dag_hash.is_empty(), "dag_hash must be set");

    let resp = admin_call(leader_addr, AdminRequest::JobInspect(job_id)).await;
    let detail = match resp {
        AdminResponse::JobDetail(d) => d,
        other => panic!("expected JobDetail, got {other:?}"),
    }
    .unwrap_or_else(|| panic!("JobInspect({job_id}) must return Some"));
    assert_eq!(detail.job_id, job_id);
    assert_eq!(detail.tasks.len(), 1);
    assert_eq!(detail.tasks[0].job_id, job_id);
    assert_eq!(
        detail.dag_hash, summary.dag_hash,
        "JobInspect dag_hash must match ListJobs dag_hash"
    );

    drop(tc);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_events_round_trip_through_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("bee-client.sqlite");
    let database = db::Database::open(&db_path).expect("Database::open must succeed");

    let id_ds = {
        let conn = database.lock().expect("db lock");
        audit::record(
            &conn,
            NewAuditEvent {
                actor: "test",
                action: "datasource.create",
                result: "Success",
                summary: "registered binance",
                resource_kind: Some("datasource"),
                resource_id: Some("binance"),
                application_id: None,
                correlation_id: None,
                operation_id: None,
                nav_kind: None,
                nav_resource_id: None,
            },
        )
        .expect("audit::record datasource")
    };
    let id_pl = {
        let conn = database.lock().expect("db lock");
        audit::record(
            &conn,
            NewAuditEvent {
                actor: "test",
                action: "pipeline.deploy",
                result: "Success",
                summary: "deployed SELECT 1",
                resource_kind: Some("pipeline"),
                resource_id: Some("select-1"),
                application_id: None,
                correlation_id: None,
                operation_id: None,
                nav_kind: None,
                nav_resource_id: None,
            },
        )
        .expect("audit::record pipeline")
    };
    assert!(id_ds > 0 && id_pl > 0 && id_ds != id_pl);

    let conn = database.lock().expect("db lock");
    let events = audit::list(&conn, 100).expect("audit::list");
    assert_eq!(events.len(), 2, "two audit events expected");

    let ds = events
        .iter()
        .find(|e| e.action == "datasource.create")
        .expect("datasource event");
    assert_eq!(ds.result, "Success");
    assert_eq!(ds.resource_kind.as_deref(), Some("datasource"));
    assert_eq!(ds.resource_id.as_deref(), Some("binance"));

    let pl = events
        .iter()
        .find(|e| e.action == "pipeline.deploy")
        .expect("pipeline event");
    assert_eq!(pl.result, "Success");
    assert_eq!(pl.resource_kind.as_deref(), Some("pipeline"));
    assert_eq!(pl.resource_id.as_deref(), Some("select-1"));
}