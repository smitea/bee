//! S21 close-out: release() is called on terminal lifecycle transition.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bee_control::JobLifecycleState;
use bee_control::kv::{Op, TaskStatus};
use bee_control::raft::cluster::{Cluster, ClusterConfig};
use bee_plugin_sdk::{PluginManifest, PluginName};
use bee_registry::PluginManager;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn release_on_completed_lifecycle_unloads_plugin() {
    let pm = Arc::new(Mutex::new(PluginManager::new()));
    let cluster = Cluster::new(ClusterConfig {
        plugin_manager: Some(pm.clone()),
        ..ClusterConfig::default()
    })
    .await;
    cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader elected");
    let leader = cluster.leader().await.unwrap();

    // Register Plugin X. PluginManager::register takes
    // `&[u8]` content (for the SHA-256 hash) — use a stable
    // byte slice as the source.
    let id = {
        let mut mgr = pm.lock().unwrap();
        mgr.register(
            b"p1".as_slice(),
            PluginManifest {
                name: PluginName("p1".into()),
                feature_version: "1.0.0".into(),
                abi_version: "v1".into(),
                adapters: vec![],
                handlers: vec![],
            },
        )
        .expect("register plugin")
    };
    assert!(pm.lock().unwrap().retain(&id));
    assert_eq!(pm.lock().unwrap().refcount_of(&id), Some(1));

    // Register a Job that uses Plugin X.
    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 1,
                dag_hash: "j1".into(),
                owner_node: leader,
                tenant: 0,
                dependencies: vec![],
                plugins: std::iter::once(id.clone()).collect(),
            },
        )
        .await
        .expect("register job");

    // Add a task to make the SM accept the lifecycle update.
    cluster
        .submit(
            leader,
            Op::RegisterTask {
                task_id: 1,
                job_id: 1,
                phase_id: 0,
                owner_node: leader,
                status: TaskStatus::Running,
                started_at_ms: 0,
            },
        )
        .await
        .expect("register task");

    // Transition the Job to Completed.
    cluster
        .submit(
            leader,
            Op::UpdateJobLifecycle {
                job_id: 1,
                state: JobLifecycleState::Completed,
            },
        )
        .await
        .expect("update lifecycle");

    // The plugin should be auto-unloaded (refcount dropped to 0).
    assert_eq!(pm.lock().unwrap().refcount_of(&id), None, "plugin should auto-unload");
}
