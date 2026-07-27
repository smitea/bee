//! S17: Producer Pipeline detection at deploy.
//!
//! `bee_deploy_local` (S49) is wired (in this same change) to
//! scan the SQL for `EMIT INTO <plugin>` / `CREATE SINK <plugin>`
//! and register the Job as a Producer via
//! `Op::RegisterDatasourceProducer`. Subscriber detection is
//! gated on S18's cross-Pipeline SQL syntax (deferred).
//!
//! The existing `job_mode()` derivation at view time picks up
//! the new producer entry and renders the Job as `Producer` in
//! `bee jobs list`.

use std::collections::BTreeMap;

use bee_control::control_plane::{ControlPlaneStateMachine, JobMode};
use bee_control::kv::Op;
use bee_control::signature::stream_signature;

#[test]
fn job_with_emit_into_plugin_is_classified_as_producer() {
    // S17 acceptance: a Job that emits to a plugin (via
    // `EMIT INTO foo` or `CREATE SINK foo`) is classified as
    // `Producer`. We test the SM-level derivation here; the
    // full `bee_deploy_local` CLI flow is exercised manually
    // (the deploy path is one process; the CP is per-process).
    let mut cp = ControlPlaneStateMachine::new(std::sync::Arc::new(std::sync::Mutex::new(bee_registry::PluginManager::new())));

    // Register a Job.
    cp.apply_op(&Op::RegisterJob {
        job_id: 1,
        dag_hash: "demo".into(),
        owner_node: 1,
        tenant: 0,
        plugins: vec![],
        dependencies: vec![]
    })
    .unwrap();

    // Mark the Job as Producer of the "binance" stream. Use
    // the StreamSignature for `(binance, emit, {})` — a
    // placeholder; a future story (S18.x) threads the actual
    // per-call args through the signature.
    let sig = stream_signature("binance", "emit", &BTreeMap::new());
    cp.apply_op(&Op::RegisterDatasourceProducer {
        signature: sig,
        job_id: 1,
    })
    .unwrap();

    // The view-time job_mode() derives Producer.
    assert_eq!(cp.job_mode(1), JobMode::Producer);
}

#[test]
fn job_without_emit_into_plugin_is_classified_as_independent() {
    // The "plain" SQL case: a Job with no `EMIT INTO <plugin>` /
    // `CREATE SINK <plugin>` is `Independent` (the default).
    let mut cp = ControlPlaneStateMachine::new(std::sync::Arc::new(std::sync::Mutex::new(bee_registry::PluginManager::new())));
    cp.apply_op(&Op::RegisterJob {
        job_id: 1,
        dag_hash: "plain".into(),
        owner_node: 1,
        tenant: 0,
        plugins: vec![],
        dependencies: vec![]
    })
    .unwrap();
    assert_eq!(cp.job_mode(1), JobMode::Independent);
}

#[test]
fn second_deploy_for_same_stream_is_idempotent() {
    // S17 acceptance note: "First writer wins. Subsequent deploys
    // with the same signature become Subscribers pointing at the
    // existing producer (ADR-0003)." The SM does NOT yet
    // auto-create a Subscriber; the second deploy's
    // RegisterDatasourceProducer is a no-op (the Vacant-entry
    // check skips it). The Job is still its own Independent Job
    // at the CP level; the S17.x follow-up wires the Subscriber
    // detection.
    //
    // This test locks down "subsequent deploy is idempotent" so
    // we don't regress while building toward S18.
    let mut cp = ControlPlaneStateMachine::new(std::sync::Arc::new(std::sync::Mutex::new(bee_registry::PluginManager::new())));
    cp.apply_op(&Op::RegisterJob {
        job_id: 1,
        dag_hash: "first".into(),
        owner_node: 1,
        tenant: 0,
        plugins: vec![],
        dependencies: vec![]
    })
    .unwrap();
    cp.apply_op(&Op::RegisterJob {
        job_id: 2,
        dag_hash: "second".into(),
        owner_node: 1,
        tenant: 0,
        plugins: vec![],
        dependencies: vec![]
    })
    .unwrap();
    let sig = stream_signature("binance", "emit", &BTreeMap::new());
    cp.apply_op(&Op::RegisterDatasourceProducer {
        signature: sig.clone(),
        job_id: 1,
    })
    .unwrap();
    // Second deploy with the same signature: the Vacant check
    // skips the insert (Job 1 stays as the producer).
    cp.apply_op(&Op::RegisterDatasourceProducer {
        signature: sig,
        job_id: 2,
    })
    .unwrap();
    // Verify: Job 1 is still the producer (Job 2 is a separate
    // Independent Job — the second deploy is a no-op for the
    // producer registration but the Job itself is registered).
    assert_eq!(cp.job_mode(1), JobMode::Producer);
    assert_eq!(cp.job_mode(2), JobMode::Independent);
}
