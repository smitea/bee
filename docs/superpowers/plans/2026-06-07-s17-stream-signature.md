# S17 · StreamSignature + Producer/Subscriber + reconnect Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement S17 end-to-end: `StreamSignature` sha256 hash + `Deployer` registers Producer (or becomes Subscriber) by signature + CP propagation when Producer dies + `bee jobs list` Mode column + `StreamSubscriber` runtime state machine for reconnect.

**Architecture:** Five layers, each test-driven and self-contained. New module `crates/bee-control/src/signature.rs` owns the hash. Existing `Op::RegisterDatasourceProducer` + `datasource_producers` registry in `control_plane.rs` is reused; one new `propagate_producer_death` method. `Deployer::deploy` gains a pre-`RegisterJob` step that computes signatures, looks up existing Producers, and either registers or adds a `DependencyRecord`. `jobs_view.rs` gains a `JobMode` enum + `Mode` column. `crates/bee-runtime/src/subscriber.rs` is new: pure state machine + CP-watcher for reconnect. BRP wire re-subscribe is a `todo!()` with a follow-up issue.

**Tech Stack:** Rust 2021, `sha2 = "0.10"`, `hex = "0.4"`, `serde_json = "1"` (new deps for `bee-control`); `tokio` (existing); `Cluster` test harness from `crates/bee-control/tests/raft_cluster.rs` for integration tests.

**Reference docs:**
- Design: `docs/superpowers/specs/2026-06-07-s17-stream-signature-design.md` (§1–§7)
- Story acceptance: `docs/stories.md` §S17 (lines 483–503)
- ADR-0011: `docs/adr/0011-stream-identity-and-backfill.md` (uncommitted, in working tree)

**Pre-flight (read these before starting):**
- `CONTEXT.md` — domain vocabulary (StreamSignature, Producer, Subscriber, Datasource)
- `crates/bee-control/src/kv.rs:78-104` — existing `Op` enum
- `crates/bee-control/src/control_plane.rs:78-90, 196-206, 263-281` — existing `datasource_producers` field, apply path, lookup
- `crates/bee-control/src/deployer.rs` — `Deployer::deploy` stub at line 184
- `crates/bee-dsl-sql/src/preprocess.rs:71-100, 165-217` — `UseDirective`, `scan_dot_calls`, `preprocess`
- `crates/bee-control/src/jobs_view.rs` — existing view rendering (lines 127-135 for format_lifecycle)
- `crates/bee-control/tests/raft_cluster.rs` — `Cluster` test harness pattern

**Working-tree state (do not touch):** The working tree contains uncommitted S33 work (5 mock plugin scaffolds under `plugins/`, ADR-0011, `Cargo.toml` workspace members, `README.md` / `docs/product-design.md` / `docs/adr/README.md` updates, `docs/stories.md` S33-S41 additions). Leave it untouched — this plan layers S17 on top of `main` HEAD without disturbing those files.

---

## File structure

**New files:**
- `crates/bee-control/src/signature.rs` — `stream_signature` hash function (with `#[cfg(test)] mod tests`)
- `crates/bee-runtime/src/subscriber.rs` — `StreamSubscriber` state machine (with `#[cfg(test)] mod tests`)
- `crates/bee-control/tests/signature_integration.rs` — Cluster-level hash idempotency
- `crates/bee-control/tests/deployer_s17.rs` — Deployer end-to-end
- `crates/bee-runtime/tests/subscriber.rs` — Cluster-level reconnect

**Modified files:**
- `crates/bee-control/Cargo.toml` — add `sha2`, `hex`, `serde_json`
- `crates/bee-control/src/lib.rs` — re-export `signature`
- `crates/bee-control/src/control_plane.rs` — add `propagate_producer_death`
- `crates/bee-control/src/deployer.rs` — wire signature lookup + Producer/Subscriber decision
- `crates/bee-control/src/jobs_view.rs` — add `JobMode` enum + `Mode` column
- `crates/bee-runtime/src/lib.rs` — re-export `subscriber`
- `crates/bee-runtime/Cargo.toml` — add `bee-control` dep (for `JobLifecycleState` + `ControlPlaneHandle` types)
- `crates/bee-control/tests/producer_subscriber.rs` — extend with 3 propagation tests
- `crates/bee-control/tests/jobs_view.rs` — extend with 3 Mode tests

**Boundary responsibilities:**
- `signature.rs`: pure function, no I/O, no async, no Bee types
- `control_plane.rs::propagate_producer_death`: SM-only, no async, no I/O
- `deployer.rs::deploy`: async, calls into `control_plane` + `signature`, no new traits
- `jobs_view.rs::job_mode`: SM-only view helper
- `subscriber.rs::StreamSubscriber`: pure state machine; the `tick` function is sync and unit-testable

---

## Task 1: Add `sha2`, `hex`, `serde_json` to `bee-control/Cargo.toml`

**Files:**
- Modify: `crates/bee-control/Cargo.toml`

- [ ] **Step 1: Open `crates/bee-control/Cargo.toml` and add three deps**

Current file (lines 1-17):
```toml
[package]
name = "bee-control"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true
description = "Bee 控制面: Raft 客户端 + 调度器 + Work-Stealing 仲裁 (Raft 集群仲裁的所有权变更)"

[dependencies]
tokio = { workspace = true }
bee-runtime = { workspace = true }
bee-plugin-sdk = { workspace = true }
bee-dsl-sql = { workspace = true }
thiserror = "2"

[lints]
workspace = true
```

Replace the `[dependencies]` block with:
```toml
[dependencies]
tokio = { workspace = true }
bee-runtime = { workspace = true }
bee-plugin-sdk = { workspace = true }
bee-dsl-sql = { workspace = true }
thiserror = "2"
sha2 = "0.10"
hex = "0.4"
serde_json = "1"
```

- [ ] **Step 2: Verify `cargo check` passes**

Run:
```bash
cd /Users/shaw/Developer/rust/bee && cargo check -p bee-control 2>&1 | tail -20
```

Expected: `Finished` line, no errors. New deps are downloaded and resolved.

- [ ] **Step 3: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-control/Cargo.toml && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §0: add sha2, hex, serde_json to bee-control for StreamSignature"
```

---

## Task 2: §1 StreamSignature — write the failing test (RED)

**Files:**
- Create: `crates/bee-control/src/signature.rs`

- [ ] **Step 1: Create the file with only the test module**

```rust
// crates/bee-control/src/signature.rs

//! S17 §1: canonical hash of a Stream's identity (ADR-0011).
//!
//! `StreamSignature = hex(sha256(name || ":" || method || ":" || inner))`
//! where `inner = hex(sha256(canonical_json(stream_topology_args)))`.
//!
//! BTreeMap serialization is key-sorted, so `serde_json::to_string`
//! is canonical for `BTreeMap<String, String>`.
//!
//! This module is pure: no I/O, no async, no Bee types. Anything
//! that needs to fingerprint a Stream — deployer, control plane,
//! jobs view — funnels through this single function.

use std::collections::BTreeMap;
use sha2::{Digest, Sha256};

/// Compute the StreamSignature for a given (datasource, method, args)
/// triple per ADR-0011. Returns 64 lowercase hex chars.
pub fn stream_signature(
    datasource_name: &str,
    adapter_method: &str,
    stream_topology_args: &BTreeMap<String, String>,
) -> String {
    // Intentional TDD placeholder: this body fails all 6 tests below.
    // The real implementation lands in Task 3 (GREEN).
    let _ = (datasource_name, adapter_method, stream_topology_args);
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn deterministic_for_same_inputs() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "5min")]));
        let b = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "5min")]));
        assert_eq!(a, b, "same inputs must hash to the same value");
    }

    #[test]
    fn different_datasource_yields_different_signature() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT")]));
        let b = stream_signature("google_news", "search",
            &args(&[("query", "btc")]));
        assert_ne!(a, b);
        assert!(!a.is_empty() && !b.is_empty(),
            "both must produce non-empty signatures");
    }

    #[test]
    fn different_method_yields_different_signature() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT")]));
        let b = stream_signature("binance", "emit",
            &args(&[("symbol", "BTC/USDT")]));
        assert_ne!(a, b);
    }

    #[test]
    fn different_args_yield_different_signature() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "5min")]));
        let b = stream_signature("binance", "subscribe",
            &args(&[("symbol", "ETH/USDT"), ("interval", "5min")]));
        let c = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "1min")]));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn empty_args_yields_valid_signature() {
        let s = stream_signature("binance", "ping", &BTreeMap::new());
        // 64 hex chars = 32 bytes = sha256 output
        assert_eq!(s.len(), 64,
            "sha256 hex must be 64 chars, got {} chars: {s:?}", s.len());
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()),
            "must be lowercase hex, got: {s:?}");
    }

    #[test]
    fn arg_order_does_not_matter() {
        // BTreeMap gives the same key order regardless of insertion order.
        let mut a = BTreeMap::new();
        a.insert("symbol".to_string(), "BTC/USDT".to_string());
        a.insert("interval".to_string(), "5min".to_string());
        let mut b = BTreeMap::new();
        b.insert("interval".to_string(), "5min".to_string());
        b.insert("symbol".to_string(), "BTC/USDT".to_string());
        let sa = stream_signature("binance", "subscribe", &a);
        let sb = stream_signature("binance", "subscribe", &b);
        assert_eq!(sa, sb);
    }
}
```

- [ ] **Step 2: Re-export `signature` from `crates/bee-control/src/lib.rs`**

Current file (lines 1-36 of `crates/bee-control/src/lib.rs`):
```rust
mod builtin_handlers;
mod cluster_status;
mod control_plane;
mod datasource;
mod deployer;
mod diagnostics_view;
mod heartbeat;
mod jobs_view;
mod kv;
pub mod raft;
mod rebalancer;
mod scheduler;
mod secret_store;
mod worker;
```

Add `pub mod signature;` after `mod deployer;` and before `mod diagnostics_view;`:
```rust
mod builtin_handlers;
mod cluster_status;
mod control_plane;
mod datasource;
mod deployer;
pub mod signature;
mod diagnostics_view;
mod heartbeat;
mod jobs_view;
mod kv;
pub mod raft;
mod rebalancer;
mod scheduler;
mod secret_store;
mod worker;
```

(Keep `mod signature;` as `pub mod` so integration tests in `tests/` can reach it via `bee_control::signature::stream_signature`.)

- [ ] **Step 3: Run the tests; they should all FAIL**

Run:
```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control --lib signature:: 2>&1 | tail -40
```

Expected: 6 failed tests, with messages about empty strings. For example `deterministic_for_same_inputs` fails because both `a` and `b` are empty strings (which happen to be equal, but the other 5 tests assert non-empty).

Actually — `deterministic_for_same_inputs` will **pass** with the empty-string placeholder (empty == empty). That's a known TDD caveat. Mark it as the "test that should pass trivially" and focus on the 5 other tests failing.

To be strict, change `assert_eq!(a, b)` in `deterministic_for_same_inputs` to also assert `!a.is_empty()`:
```rust
    #[test]
    fn deterministic_for_same_inputs() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "5min")]));
        let b = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "5min")]));
        assert!(!a.is_empty(), "must produce a non-empty signature");
        assert_eq!(a, b, "same inputs must hash to the same value");
    }
```

After this change, the 6 tests will all fail.

- [ ] **Step 4: Commit (RED)**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-control/src/signature.rs crates/bee-control/src/lib.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §1 RED: 6 unit tests for stream_signature (all failing)"
```

---

## Task 3: §1 StreamSignature — implement and pass the test (GREEN)

**Files:**
- Modify: `crates/bee-control/src/signature.rs` (only the `stream_signature` body)

- [ ] **Step 1: Replace the placeholder body with the real implementation**

Replace the body of `stream_signature` (currently:
```rust
    // Intentional TDD placeholder: this body fails all 6 tests below.
    // The real implementation lands in Task 3 (GREEN).
    let _ = (datasource_name, adapter_method, stream_topology_args);
    String::new()
```
) with:
```rust
    // inner: sha256 over the canonical JSON of the topology args.
    // BTreeMap<String, String> serializes in key-sorted order, so
    // serde_json::to_string is canonical for free.
    let inner = {
        let json = serde_json::to_string(stream_topology_args)
            .expect("BTreeMap<String, String> is always serializable");
        hex::encode(Sha256::digest(json.as_bytes()))
    };
    // outer: sha256 over `name ":" method ":" inner`
    let mut h = Sha256::new();
    h.update(datasource_name.as_bytes());
    h.update(b":");
    h.update(adapter_method.as_bytes());
    h.update(b":");
    h.update(inner.as_bytes());
    hex::encode(h.finalize())
```

- [ ] **Step 2: Run the tests; they should all PASS**

Run:
```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control --lib signature:: 2>&1 | tail -15
```

Expected: `test result: ok. 6 passed; 0 failed`.

- [ ] **Step 3: Commit (GREEN)**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-control/src/signature.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §1 GREEN: implement stream_signature (canonical sha256 per ADR-0011)"
```

---

## Task 4: §3 CP propagation — write the failing test (RED)

**Files:**
- Modify: `crates/bee-control/tests/producer_subscriber.rs` (extend with 3 tests)

- [ ] **Step 1: Append 3 new test cases to the end of the file**

Current file ends at line 173 with `different_signatures_get_different_producers`. Add the following to the end of the file (after the closing `}` of that test):

```rust
// ---- S17 §3: propagate_producer_death ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn producer_death_flips_running_subscribers_to_waiting() {
    use bee_control::kv::{JobLifecycleState, Op};

    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    // Register Job A as a Producer for a signature.
    let sig = "binance:BTC/USDT:5m".to_string();
    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer { signature: sig.clone(), job_id: 1 },
        )
        .await
        .expect("register producer A");
    wait_for_producer(&cluster, &sig, 1, Duration::from_secs(2)).await;

    // Register Job B with a dependency on A (Subscriber pattern).
    cluster
        .submit(
            leader,
            Op::RegisterJob {
                job_id: 2,
                dag_hash: "dag-b".into(),
                owner_node: leader,
                tenant: 0,
            },
        )
        .await
        .expect("register job B");
    cluster
        .submit(
            leader,
            Op::UpdateJobLifecycle { job_id: 2, state: JobLifecycleState::Running },
        )
        .await
        .expect("B -> Running");
    cluster
        .submit(
            leader,
            Op::RegisterDependency {
                downstream_job: 2,
                upstream_job: 1,
                stream: sig.clone(),
            },
        )
        .await
        .expect("register B's dep on A");

    // Now propagate A's death.
    let flipped = {
        let handle = cluster.handle(leader).expect("handle");
        let mut cp = handle.cp.lock().await;
        cp.propagate_producer_death(1)
    };

    assert_eq!(flipped, vec![2], "Job B should have been flipped");

    // Verify B's lifecycle is now WaitingForUpstream.
    let handle = cluster.handle(leader).expect("handle");
    let cp = handle.cp.lock().await;
    assert_eq!(cp.list_jobs()[0].lifecycle, JobLifecycleState::WaitingForUpstream);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn producer_death_is_noop_on_already_waiting_subscribers() {
    use bee_control::kv::{JobLifecycleState, Op};

    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    cluster
        .submit(
            leader,
            Op::RegisterDatasourceProducer { signature: "sig".into(), job_id: 1 },
        )
        .await
        .expect("register A");
    cluster
        .submit(leader, Op::RegisterJob {
            job_id: 2, dag_hash: "d".into(), owner_node: leader, tenant: 0,
        })
        .await
        .expect("register B");
    cluster
        .submit(leader, Op::UpdateJobLifecycle { job_id: 2, state: JobLifecycleState::WaitingForUpstream })
        .await
        .expect("B -> WaitingForUpstream");
    cluster
        .submit(leader, Op::RegisterDependency {
            downstream_job: 2, upstream_job: 1, stream: "sig".into(),
        })
        .await
        .expect("register dep");

    let flipped = {
        let handle = cluster.handle(leader).expect("handle");
        let mut cp = handle.cp.lock().await;
        cp.propagate_producer_death(1)
    };

    assert!(flipped.is_empty(), "B is already WaitingForUpstream; no flip");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn producer_death_does_not_touch_unrelated_running_jobs() {
    use bee_control::kv::{JobLifecycleState, Op};

    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    // A is the Producer, B depends on A (Subscriber), C is unrelated.
    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: "s".into(), job_id: 1 })
        .await.expect("");
    cluster.submit(leader, Op::RegisterJob { job_id: 2, dag_hash: "d".into(), owner_node: leader, tenant: 0 })
        .await.expect("");
    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 2, state: JobLifecycleState::Running })
        .await.expect("");
    cluster.submit(leader, Op::RegisterDependency { downstream_job: 2, upstream_job: 1, stream: "s".into() })
        .await.expect("");

    cluster.submit(leader, Op::RegisterJob { job_id: 3, dag_hash: "c".into(), owner_node: leader, tenant: 0 })
        .await.expect("");
    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 3, state: JobLifecycleState::Running })
        .await.expect("");

    let flipped = {
        let handle = cluster.handle(leader).expect("handle");
        let mut cp = handle.cp.lock().await;
        cp.propagate_producer_death(1)
    };

    assert_eq!(flipped, vec![2], "only B should flip; C is unrelated");

    let handle = cluster.handle(leader).expect("handle");
    let cp = handle.cp.lock().await;
    let c = cp.list_jobs().iter().find(|j| j.job_id == 3).expect("C exists");
    assert_eq!(c.lifecycle, JobLifecycleState::Running, "C must remain Running");
}
```

- [ ] **Step 2: Run the new tests; they should FAIL (no `propagate_producer_death` exists yet)**

Run:
```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control --test producer_subscriber 2>&1 | tail -30
```

Expected: The 3 new tests fail with `error[E0599]: no function or associated method named 'propagate_producer_death' found for struct 'ControlPlaneStateMachine'`. The 3 existing tests still pass.

- [ ] **Step 3: Commit (RED)**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-control/tests/producer_subscriber.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §3 RED: 3 propagation tests (all failing — method does not exist)"
```

---

## Task 5: §3 CP propagation — implement and pass the test (GREEN)

**Files:**
- Modify: `crates/bee-control/src/control_plane.rs` (add one method)

- [ ] **Step 1: Add `propagate_producer_death` to `impl ControlPlaneStateMachine`**

Open `crates/bee-control/src/control_plane.rs`. Find the existing `pub fn list_jobs` method (around line 263) and add the new method right after `datasource_producer_count` (around line 281, the last method on the impl block). The exact insertion point is **after** this block (around line 282):

```rust
    pub fn datasource_producer_count(&self) -> usize {
        self.datasource_producers.len()
    }
```

Append immediately after:
```rust
    /// S17 §3: when a Producer Job dies (Failed / Completed /
    /// removed), all Subscribers (Jobs whose `dependencies`
    /// list contains `upstream_job == producer_job_id`) must flip
    /// from `Running` to `WaitingForUpstream`. Returns the list of
    /// JobIds that were flipped, for the orchestrator's log.
    ///
    /// Idempotent: subscribers that are already
    /// `WaitingForUpstream` (or any non-`Running` state) are not
    /// touched, even if they still have a dependency on the dead
    /// producer.
    pub fn propagate_producer_death(
        &mut self,
        producer_job_id: u32,
    ) -> Vec<u32> {
        let mut flipped = vec![];
        for (job_id, job) in self.jobs.iter_mut() {
            let depends_on_dead = job
                .dependencies
                .iter()
                .any(|d| d.upstream_job == producer_job_id);
            if depends_on_dead
                && job.lifecycle == JobLifecycleState::Running
            {
                job.lifecycle = JobLifecycleState::WaitingForUpstream;
                flipped.push(*job_id);
            }
        }
        flipped
    }
```

- [ ] **Step 2: Run the propagation tests; they should PASS**

Run:
```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control --test producer_subscriber 2>&1 | tail -20
```

Expected: `test result: ok. 6 passed; 0 failed` (3 existing + 3 new).

- [ ] **Step 3: Run the full bee-control test suite to confirm no regression**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit (GREEN)**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-control/src/control_plane.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §3 GREEN: propagate_producer_death — Subscribers flip to WaitingForUpstream"
```

---

## Task 6: §4 jobs_view Mode — write the failing test (RED)

**Files:**
- Modify: `crates/bee-control/tests/jobs_view.rs` (extend; create if not present)

- [ ] **Step 1: Read the existing `jobs_view.rs` test file to find the right extension point**

Run:
```bash
ls -la /Users/shaw/Developer/rust/bee/crates/bee-control/tests/jobs_view.rs
```

If the file does not exist, create it with the 3 test cases below (Step 2). If it exists, append the 3 test cases from Step 2 to the end of the file.

- [ ] **Step 2: Add 3 new tests covering `JobMode` derivation**

If creating a new file, write:
```rust
// crates/bee-control/tests/jobs_view.rs

use std::time::Duration;
use bee_control::control_plane::JobMode;
use bee_control::kv::{JobLifecycleState, Op};
use bee_control::raft::cluster::{Cluster, ClusterConfig};

async fn fresh_cluster() -> Cluster {
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster.wait_for_leader(Duration::from_secs(5)).await.expect("leader");
    cluster
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_mode_distinguishes_producer_subscriber_independent() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    // Job 1: Producer
    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: "sig-A".into(), job_id: 1 })
        .await.expect("");
    // Job 2: Subscriber (depends on 1)
    cluster.submit(leader, Op::RegisterJob { job_id: 2, dag_hash: "d".into(), owner_node: leader, tenant: 0 })
        .await.expect("");
    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 2, state: JobLifecycleState::Running })
        .await.expect("");
    cluster.submit(leader, Op::RegisterDependency { downstream_job: 2, upstream_job: 1, stream: "sig-A".into() })
        .await.expect("");
    // Job 3: Independent
    cluster.submit(leader, Op::RegisterJob { job_id: 3, dag_hash: "i".into(), owner_node: leader, tenant: 0 })
        .await.expect("");

    let handle = cluster.handle(leader).expect("handle");
    let cp = handle.cp.lock().await;
    assert_eq!(cp.job_mode(1), JobMode::Producer);
    assert_eq!(cp.job_mode(2), JobMode::Subscriber);
    assert_eq!(cp.job_mode(3), JobMode::Independent);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_mode_producer_wins_when_chained() {
    // Job 1 is itself a Producer AND has a dep on another Producer.
    // Producer should win (the Job produces its own stream).
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: "outer".into(), job_id: 1 })
        .await.expect("");
    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: "inner".into(), job_id: 2 })
        .await.expect("");
    cluster.submit(leader, Op::RegisterJob { job_id: 1, dag_hash: "d".into(), owner_node: leader, tenant: 0 })
        .await.expect("");
    cluster.submit(leader, Op::RegisterDependency { downstream_job: 1, upstream_job: 2, stream: "inner".into() })
        .await.expect("");

    let handle = cluster.handle(leader).expect("handle");
    let cp = handle.cp.lock().await;
    assert_eq!(cp.job_mode(1), JobMode::Producer,
        "Job 1 is a Producer (own signature) even though it also subscribes");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_mode_unknown_job_returns_independent() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let handle = cluster.handle(leader).expect("handle");
    let cp = handle.cp.lock().await;
    assert_eq!(cp.job_mode(999), JobMode::Independent,
        "a JobId with no record is treated as Independent (defensive default)");
}
```

- [ ] **Step 3: Run the new tests; they should FAIL (no `JobMode` / `job_mode` exists yet)**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control --test jobs_view 2>&1 | tail -20
```

Expected: compile error `error[E0432]: unresolved import 'bee_control::control_plane::JobMode'`.

- [ ] **Step 4: Commit (RED)**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-control/tests/jobs_view.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §4 RED: 3 jobs_view Mode tests (failing — JobMode does not exist)"
```

---

## Task 7: §4 jobs_view Mode — implement and pass the test (GREEN)

**Files:**
- Modify: `crates/bee-control/src/control_plane.rs` (add `JobMode` enum + `job_mode` method; update `pub use`)

- [ ] **Step 1: Add the `JobMode` enum + `job_mode` method to `control_plane.rs`**

Open `crates/bee-control/src/control_plane.rs`. Right after the `JobLifecycleState` import block (search for `use crate::kv::{`) — actually, the file already has `use crate::kv::{...}` somewhere; the new enum does not need to import anything from `kv`. Add the enum and method after the existing `propagate_producer_death` block (added in Task 5).

Add the `JobMode` enum definition (above the `impl ControlPlaneStateMachine` block — find `impl ControlPlaneStateMachine {` and insert the enum definition just before it):

```rust
/// S17 §4: a Job's role with respect to Stream sharing.
/// - `Producer`: this JobId appears in the
///   `datasource_producers` registry (it is the canonical owner
///   of a Stream).
/// - `Subscriber`: this Job has at least one dependency whose
///   `upstream_job` is a Producer (it consumes a Stream owned by
///   another Job).
/// - `Independent`: neither — the Job is a normal, self-contained
///   Pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMode {
    Producer,
    Subscriber,
    Independent,
}
```

Then add `job_mode` to the `impl ControlPlaneStateMachine` block (after `propagate_producer_death`):
```rust
    /// S17 §4: derive a Job's [`JobMode`] at view time. A JobId
    /// with no record returns `Independent` (defensive default
    /// for views that may be queried before all jobs are
    /// registered).
    pub fn job_mode(&self, job_id: u32) -> JobMode {
        if self.datasource_producers.values().any(|&p| p == job_id) {
            return JobMode::Producer;
        }
        if let Some(job) = self.jobs.get(&job_id) {
            for d in &job.dependencies {
                if self.datasource_producers.values().any(|&p| p == d.upstream_job) {
                    return JobMode::Subscriber;
                }
            }
        }
        JobMode::Independent
    }
```

- [ ] **Step 2: Re-export `JobMode` from `crates/bee-control/src/control_plane.rs`**

The tests import via `bee_control::control_plane::JobMode`, so the existing `pub fn` on `control_plane` is enough — no extra `pub use` needed because `JobMode` is defined in the module and is `pub`. Verify by running `cargo test`.

- [ ] **Step 3: Run the jobs_view tests; they should PASS**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control --test jobs_view 2>&1 | tail -15
```

Expected: `test result: ok. 3 passed; 0 failed`.

- [ ] **Step 4: Wire the Mode column into the existing `jobs_view` rendering**

Open `crates/bee-control/src/jobs_view.rs`. The current rendering iterates over jobs and prints columns including lifecycle, status, etc. Add a new column header `MODE` and a per-job `Mode` cell.

The exact line numbers depend on the current state. Locate the `format_lifecycle` function (around line 127) and the loop that prints jobs. Add a new helper next to `format_lifecycle`:

```rust
fn format_mode(m: crate::control_plane::JobMode) -> String {
    use crate::control_plane::JobMode::*;
    match m {
        Producer => "Producer".to_string(),
        Subscriber => "Subscriber".to_string(),
        Independent => "-".to_string(),
    }
}
```

Then in the per-job print loop, add a cell that calls `format_mode(handle.cp.lock().await.job_mode(job_id))` (or whatever the current access pattern is — read the existing loop first to copy the locking style).

If the current jobs_view takes a snapshot of the CP state outside the loop, add a `Vec<JobMode>` to the snapshot struct computed at snapshot time:
```rust
let modes: Vec<JobMode> = snapshot.jobs.iter()
    .map(|j| cp.job_mode(j.job_id))
    .collect();
```

- [ ] **Step 5: Run the full bee-control test suite**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control 2>&1 | tail -10
```

Expected: all tests pass, no warnings.

- [ ] **Step 6: Commit (GREEN)**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-control/src/control_plane.rs crates/bee-control/src/jobs_view.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §4 GREEN: JobMode + job_mode + Mode column in jobs_view"
```

---

## Task 8: §2 Deployer wiring — write the failing test (RED)

**Files:**
- Create: `crates/bee-control/tests/deployer_s17.rs`

- [ ] **Step 1: Read `crates/bee-control/src/deployer.rs` to confirm the current shape of `Pipeline` and `Deployer`**

```bash
grep -n "pub struct Pipeline\|pub struct Deployer\|impl Deployer\|pub fn deploy\|pub trait Pipeline" /Users/shaw/Developer/rust/bee/crates/bee-control/src/deployer.rs /Users/shaw/Developer/rust/bee/crates/bee-dsl-sql/src/*.rs
```

Verify that `Pipeline` is the DataFusion-compiled DAG and that `Deployer::deploy` takes a `Pipeline` (or `Dag`) and returns a `JobId`. Note: the deployer may be a stub (line 184 in the design) that just returns a hardcoded `JobId`. The deployer must call `Pipeline::stream_identities()` (which we'll add in Task 9) — for now, the test only requires that `Deployer::deploy` is wired to call into the signature registry.

- [ ] **Step 2: Read the existing test for `deployer.rs` (if any) to copy the `Cluster` setup pattern**

```bash
ls /Users/shaw/Developer/rust/bee/crates/bee-control/tests/deploy_pipeline.rs
```

If it exists, copy its setup helpers.

- [ ] **Step 3: Create the deployer_s17 test file with 5 failing tests**

The exact form of the `Pipeline` argument depends on the deployer's current signature. A common shape is `Deployer::deploy(pipeline: Pipeline) -> Result<JobId>`. The `Pipeline::stream_identities()` method we add in Task 9 is the interface the deployer calls into. For the RED test, we only need to assert the post-condition (CP registry contains the right entries), not the internal API. Use whatever `Pipeline` constructor is currently used in `deploy_pipeline.rs`.

```rust
// crates/bee-control/tests/deployer_s17.rs

use std::time::Duration;
use bee_control::kv::Op;
use bee_control::raft::cluster::{Cluster, ClusterConfig};

async fn fresh_cluster() -> Cluster {
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster.wait_for_leader(Duration::from_secs(5)).await.expect("leader");
    cluster
}

/// Helper: read the StreamSignature -> JobId mapping for a known
/// signature from any alive node's CP.
async fn read_producer(cluster: &Cluster, sig: &str) -> Option<u32> {
    for (id, _handle) in cluster.nodes() {
        if !cluster.is_alive(id) { continue; }
        let handle = cluster.handle(id).expect("handle");
        let cp = handle.cp.lock().await;
        if let Some(p) = cp.lookup_datasource_producer(sig) {
            return Some(p);
        }
    }
    None
}

async fn read_count(cluster: &Cluster) -> usize {
    for (id, _handle) in cluster.nodes() {
        if !cluster.is_alive(id) { continue; }
        let handle = cluster.handle(id).expect("handle");
        return handle.cp.lock().await.datasource_producer_count();
    }
    0
}

async fn wait_for_count(cluster: &Cluster, expected: usize, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if read_count(cluster).await == expected { return; }
        if tokio::time::Instant::now() >= deadline {
            panic!("count did not reach {expected} within {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_first_pipeline_becomes_producer() {
    // Use the same shape that `deploy_pipeline.rs` uses to build a Pipeline;
    // we expect to deploy a Pipeline whose stream identity is
    // ("binance", "subscribe", {"symbol": "BTC/USDT", "interval": "5min"}).
    // For the RED test we use a synthetic JobId registration that exercises
    // the same `Op::RegisterDatasourceProducer` op the deployer will submit.
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "test:binance:subscribe:abc123".to_string();
    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: sig.clone(), job_id: 1 })
        .await.expect("");
    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;
    assert_eq!(read_producer(&cluster, &sig).await, Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_second_pipeline_with_same_signature_becomes_subscriber() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "test:binance:subscribe:def456".to_string();

    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: sig.clone(), job_id: 1 })
        .await.expect("");
    // Second deployer would submit a `RegisterDependency` and NOT
    // a second `RegisterDatasourceProducer`. Verify the count stays 1.
    cluster.submit(leader, Op::RegisterJob { job_id: 2, dag_hash: "d".into(), owner_node: leader, tenant: 0 })
        .await.expect("");
    cluster.submit(leader, Op::RegisterDependency { downstream_job: 2, upstream_job: 1, stream: sig.clone() })
        .await.expect("");

    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;
    assert_eq!(read_producer(&cluster, &sig).await, Some(1),
        "Job 1 is still the sole Producer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_pipeline_with_different_args_gets_different_producer() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig_btc = "test:binance:subscribe:BTC".to_string();
    let sig_eth = "test:binance:subscribe:ETH".to_string();

    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: sig_btc.clone(), job_id: 1 })
        .await.expect("");
    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: sig_eth.clone(), job_id: 2 })
        .await.expect("");

    wait_for_count(&cluster, 2, Duration::from_secs(2)).await;
    assert_eq!(read_producer(&cluster, &sig_btc).await, Some(1));
    assert_eq!(read_producer(&cluster, &sig_eth).await, Some(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_repeat_of_same_signature_is_idempotent() {
    // Same as the second test, but explicitly: re-deploying the
    // same Pipeline (same signature) must NOT add a new Producer
    // entry and must NOT bump the count.
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "test:replay:sig".to_string();

    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: sig.clone(), job_id: 1 })
        .await.expect("");
    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;

    // Re-deploy (idempotent path: lookup hits existing entry,
    // deployer just adds a DependencyRecord; no second
    // RegisterDatasourceProducer op).
    cluster.submit(leader, Op::RegisterJob { job_id: 1, dag_hash: "d".into(), owner_node: leader, tenant: 0 })
        .await.expect("");
    cluster.submit(leader, Op::RegisterDependency { downstream_job: 1, upstream_job: 1, stream: sig.clone() })
        .await.expect("");

    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_multiple_pipelines_only_first_is_producer() {
    // 3 pipelines with the same signature: only the first is a
    // Producer; the other two are Subscribers.
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let sig = "test:multi:sig".to_string();

    for job_id in 1u32..=3 {
        if job_id == 1 {
            cluster.submit(leader, Op::RegisterDatasourceProducer { signature: sig.clone(), job_id })
                .await.expect("");
        } else {
            cluster.submit(leader, Op::RegisterJob { job_id, dag_hash: "d".into(), owner_node: leader, tenant: 0 })
                .await.expect("");
            cluster.submit(leader, Op::RegisterDependency { downstream_job: job_id, upstream_job: 1, stream: sig.clone() })
                .await.expect("");
        }
    }

    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;
    assert_eq!(read_producer(&cluster, &sig).await, Some(1));
}
```

(Note: these tests exercise the CP-level op wiring that the deployer will use. The actual end-to-end deployer test — calling `Deployer::deploy(pipeline)` with a real `Pipeline` — is added in Task 9 once `Pipeline::stream_identities()` exists. The 5 tests here give us RED coverage of the signature-driven decision logic.)

- [ ] **Step 4: Run the new tests; they should PASS at the CP level (the deployer integration test is what fails in Task 9, not these)**

Run:
```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control --test deployer_s17 2>&1 | tail -15
```

Expected: `test result: ok. 5 passed; 0 failed`. (These tests pass on the existing CP; they pin the expected behavior that the deployer will rely on.)

- [ ] **Step 5: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-control/tests/deployer_s17.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §2: 5 CP-level tests pinning signature-driven Producer/Subscriber wiring"
```

---

## Task 9: §2 Deployer wiring — implement `Pipeline::stream_identities` + deployer integration (GREEN)

**Files:**
- Modify: `crates/bee-dsl-sql/src/preprocess.rs` (add `extract_stream_identities`)
- Modify: `crates/bee-dsl-sql/src/lib.rs` (re-export)
- Modify: `crates/bee-control/src/deployer.rs` (call into the preprocessor in `deploy`)

- [ ] **Step 1: Add `extract_stream_identities` to `preprocess.rs`**

The function takes raw SQL and returns `Vec<(datasource_name, adapter_method, BTreeMap<String, String>)>`. It uses the existing `parse_use_directives` + `scan_dot_calls` helpers (lines 71-100 and 165-209 of the current `preprocess.rs`).

Add at the end of the public API section (before the `#[cfg(test)] mod tests` block):

```rust
/// S17 §2: extract the stream-producing identities from a SQL
/// Pipeline. An identity is a triple `(datasource_name,
/// adapter_method, stream_topology_args)`. The `datasource_name`
/// comes from the matching `use <name>;` directive; the
/// `adapter_method` and `stream_topology_args` come from the
/// first `<name>.<method>(<args>)` call in the body that names
/// that datasource. The function is order-preserving and
/// de-duplicated.
///
/// For the MVP, only string-typed args are captured (the
/// `BTreeMap<String, String>` map). Numeric / boolean / null
/// args are serialized to their JSON form (e.g. `100` →
/// `"100"`). Plugins that need richer topology args can
/// override the S17 signature in a follow-up.
pub fn extract_stream_identities(
    sql: &str,
) -> Vec<(String, String, std::collections::BTreeMap<String, String>)> {
    use std::collections::BTreeMap;
    let (directives, body) = match preprocess(sql) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut out: Vec<(String, String, BTreeMap<String, String>)> = vec![];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") { continue; }
        // Find a `<name>.<method>(<args>)` call.
        // Reuse scan_dot_calls' identifier pattern (hand-rolled to
        // avoid bringing the private helper into scope here).
        let bytes = line.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            let c = bytes[i];
            if !(c.is_ascii_alphabetic() || c == b'_') {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = &line[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' { j += 1; }
            if j >= bytes.len() || bytes[j] != b'.' { continue; }
            if j + 1 >= bytes.len() || !(bytes[j+1].is_ascii_alphabetic() || bytes[j+1] == b'_') {
                continue;
            }
            let method_start = j + 1;
            let mut k = method_start;
            while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
                k += 1;
            }
            let method = &line[method_start..k];
            // Skip past the call parens to the matching `)`.
            let mut paren_depth = 0;
            let mut m = k;
            let mut found_paren = false;
            while m < bytes.len() {
                match bytes[m] {
                    b'(' => { paren_depth += 1; found_paren = true; m += 1; }
                    b')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 { break; }
                        m += 1;
                    }
                    _ => m += 1,
                }
            }
            if !found_paren || paren_depth != 0 { continue; }
            let args_text = &line[k..=m.min(bytes.len()-1)];

            // Look up the matching `use` directive.
            let matched_use = directives.iter().find(|d| d.name == name);
            let _ = matched_use; // existence is enforced by strict mode in preprocess()

            // Parse args. MVP: extract `<key> => <value>` or
            // `<key> = <value>` pairs separated by `,`.
            let args_inner = args_text
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim();
            let mut map: BTreeMap<String, String> = BTreeMap::new();
            for pair in args_inner.split(',') {
                let pair = pair.trim();
                if pair.is_empty() { continue; }
                let (k_str, v_str) = if let Some(idx) = pair.find("=>") {
                    (pair[..idx].trim(), pair[idx+2..].trim())
                } else if let Some(idx) = pair.find('=') {
                    (pair[..idx].trim(), pair[idx+1..].trim())
                } else {
                    continue;
                };
                let v_str = v_str.trim_matches('\'').trim_matches('"').to_string();
                map.insert(k_str.to_string(), v_str);
            }

            // De-dupe by name+method signature.
            let dedup_key = format!("{name}.{method}");
            if !seen.insert(dedup_key) { continue; }
            out.push((name.to_string(), method.to_string(), map));
        }
    }
    out
}
```

- [ ] **Step 2: Re-export `extract_stream_identities` from `crates/bee-dsl-sql/src/lib.rs`**

Add to the `pub use preprocess::{...}` line (line 26-28):
```rust
pub use preprocess::{
    check_strict_mode, extract_stream_identities, parse_use_directives,
    preprocess, UseDirective,
};
```

- [ ] **Step 3: Add unit tests for `extract_stream_identities` to `preprocess.rs`**

At the end of the existing `#[cfg(test)] mod tests` block in `preprocess.rs`, add:

```rust
    #[test]
    fn extract_identities_finds_single_call() {
        let sql = "use binance; SELECT * FROM binance.subscribe('BTC/USDT', '5min')";
        let ids = extract_stream_identities(sql);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].0, "binance");
        assert_eq!(ids[0].1, "subscribe");
        assert_eq!(ids[0].2.get("'BTC/USDT'").map(String::as_str), Some("'BTC/USDT'"));
        // The MVP arg parser strips quotes: verify at least one of
        // the keys is BTC/USDT or 5min. The exact key name depends
        // on the parser; the existence of 2 entries is what matters.
        assert!(!ids[0].2.is_empty(), "args map should be non-empty");
    }

    #[test]
    fn extract_identities_finds_multiple_calls() {
        let sql = "use binance; use google_news; \
                   SELECT * FROM binance.subscribe('BTC/USDT', '5min') \
                   JOIN google_news.search('btc')";
        let ids = extract_stream_identities(sql);
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn extract_identities_dedupes_repeated_calls() {
        let sql = "use binance; SELECT * FROM binance.subscribe('BTC/USDT', '5min') \
                   WHERE EXISTS (SELECT 1 FROM binance.subscribe('BTC/USDT', '5min'))";
        let ids = extract_stream_identities(sql);
        assert_eq!(ids.len(), 1, "repeated calls must dedupe");
    }
```

(These test names are best-effort; if the arg-parser shape turns out different from the implementation above, adjust the test bodies — but the test names and intents stand. If a test can't be made to pass, the implementation in Step 1 has a bug; fix it there.)

- [ ] **Step 4: Run the unit tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-dsl-sql --lib preprocess:: 2>&1 | tail -15
```

Expected: tests pass (or fail with a clear message if the parser is off; iterate until green).

- [ ] **Step 5: Extend `Pipeline` with `stream_identities` + a `from_sql` constructor**

The current `Pipeline` (in `crates/bee-control/src/deployer.rs:64`) is:
```rust
pub struct Pipeline {
    name: String,
    tasks: Vec<TaskSpec>,
    edges: Vec<Edge>,
}
```

It has no SQL. S17 needs the deployer to see the SQL-derived stream identities, so extend the struct and add a `from_sql` constructor:

```rust
/// S17 §2: the (datasource_name, adapter_method, stream_topology_args)
/// triple extracted from a Pipeline's SQL. Re-exported under
/// `bee_control::deployer::StreamIdentity` for call-site clarity.
pub type StreamIdentity = (
    String,
    String,
    std::collections::BTreeMap<String, String>,
);

pub struct Pipeline {
    pub name: String,
    pub tasks: Vec<TaskSpec>,
    pub edges: Vec<Edge>,
    /// S17 §2: stream-producing identities extracted from the
    /// original SQL. Populated by `from_sql`; left empty when the
    /// Pipeline is built via the struct literal (the legacy path
    /// used by `deploy_pipeline.rs`).
    pub stream_identities: Vec<StreamIdentity>,
}

impl Pipeline {
    /// S17 §2: build a Pipeline from raw SQL. Compiles the SQL to a
    /// DAG and extracts the stream identities in one pass. Tasks /
    /// edges are populated from the compiled DAG; for the MVP we
    /// synthesize one Task per stream identity plus a single
    /// terminal Projection (the existing `linear_3` test in
    /// `deploy_pipeline.rs` is unaffected because it uses the
    /// struct literal directly, not `from_sql`).
    pub fn from_sql(
        name: &str,
        sql: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        use bee_dsl_sql::preprocess::extract_stream_identities;
        let stream_identities = extract_stream_identities(sql);
        // MVP: one Task per stream identity, plus a terminal
        // "sink" task. Edge wiring is the same as compile_to_dag
        // would produce; for the RED → GREEN path we only need
        // the deployer to see the stream_identities, so the DAG
        // shape is the simplest one that compiles. Future: wire
        // through compile_to_dag.
        let mut tasks: Vec<TaskSpec> = stream_identities
            .iter()
            .enumerate()
            .map(|(i, _)| TaskSpec {
                task_id: (i + 1) as u32,
                phase_id: i as u32,
                handler_kind: HandlerKind::Started { tag: format!("P{i}") },
                cpu_millicores: 0,
                mem_mb: 0,
            })
            .collect();
        // Add a terminal sink so the deployer has at least one
        // non-source Task.
        let sink_id = (tasks.len() + 1) as u32;
        tasks.push(TaskSpec {
            task_id: sink_id,
            phase_id: tasks.len() as u32,
            handler_kind: HandlerKind::Started { tag: "SINK".to_string() },
            cpu_millicores: 0,
            mem_mb: 0,
        });
        let mut edges: Vec<Edge> = (1..sink_id)
            .map(|i| Edge { from: i, to: i + 1 })
            .collect();
        // If there were no source tasks, drop the sink and edges.
        if stream_identities.is_empty() {
            tasks.pop();
            edges.clear();
        }
        Ok(Pipeline {
            name: name.to_string(),
            tasks,
            edges,
            stream_identities,
        })
    }
}
```

Add the import at the top of `deployer.rs`:
```rust
use bee_dsl_sql; // (already a workspace dep; the new constructor calls it)
```

- [ ] **Step 6: Wire the deployer to consult stream identities**

Open `crates/bee-control/src/deployer.rs` and modify `Deployer::deploy` (around line 184) to insert the S17 step at the start. The deployer now has access to `pipeline.stream_identities` directly, so no SQL re-parsing is needed:

```rust
    pub async fn deploy(&mut self, pipeline: Pipeline) -> Result<u32, DeployError> {
        // S17 §2: for each stream identity, look up the existing
        // Producer. If present, this Job is a Subscriber (add a
        // DependencyRecord). If not, this Job is the Producer
        // (submit RegisterDatasourceProducer).
        //
        // The Op order is: RegisterJob first (so we have a
        // job_id), then deps + producers in the same batch.
        let new_job_id = /* existing logic to allocate a job_id */;
        let stream_sigs: Vec<String> = pipeline.stream_identities
            .iter()
            .map(|(name, method, args)| {
                bee_control::signature::stream_signature(name, method, args)
            })
            .collect();

        let mut ops: Vec<Op> = vec![
            Op::RegisterJob { /* existing fields */ },
        ];
        for sig in &stream_sigs {
            if let Some(producer_id) = self.cp.lookup_datasource_producer(sig).await {
                ops.push(Op::RegisterDependency {
                    downstream_job: new_job_id,
                    upstream_job: producer_id,
                    stream: sig.clone(),
                });
            } else {
                ops.push(Op::RegisterDatasourceProducer {
                    signature: sig.clone(),
                    job_id: new_job_id,
                });
            }
        }
        // ... existing submit logic, with `ops` instead of the
        // current single RegisterJob op.
```

The exact field names in the existing `RegisterJob` call and the existing submit batch depend on the deployer's current shape. Read `deployer.rs` lines 184–250 first; the rule is "use the existing RegisterJob op and the existing submit path; the only addition is the for-loop above that appends dependency / producer ops to the batch before submission."

- [ ] **Step 7: Write the deployer end-to-end test**

Add to `crates/bee-control/tests/deployer_s17.rs` (after the 5 existing tests):

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn end_to_end_deployer_registers_producer_for_new_signature() {
    use bee_control::deployer::{Deployer, Pipeline};

    let pipeline = Pipeline::from_sql(
        "quant-btc",
        "use binance; SELECT * FROM binance.subscribe('BTC/USDT', '5min')",
    ).expect("parse pipeline");

    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let job_id = deployer.deploy(pipeline).await.expect("deploy");

    // Assert: there is exactly 1 Producer in the registry.
    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;

    // Assert: this Job's mode is Producer.
    let handle = cluster.handle(leader).expect("handle");
    let cp = handle.cp.lock().await;
    use bee_control::control_plane::JobMode;
    assert_eq!(cp.job_mode(job_id), JobMode::Producer);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn end_to_end_deployer_second_pipeline_becomes_subscriber() {
    use bee_control::deployer::{Deployer, Pipeline};

    let sql = "use binance; SELECT * FROM binance.subscribe('BTC/USDT', '5min')";
    let p1 = Pipeline::from_sql("p1", sql).expect("p1");
    let p2 = Pipeline::from_sql("p2", sql).expect("p2");

    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");
    let mut deployer = Deployer::new(DeployerConfig::default()).await;
    let job_a = deployer.deploy(p1).await.expect("deploy A");
    let job_b = deployer.deploy(p2).await.expect("deploy B");

    // Assert: count is still 1 (one Producer only).
    wait_for_count(&cluster, 1, Duration::from_secs(2)).await;

    // Assert: A is Producer, B is Subscriber.
    let handle = cluster.handle(leader).expect("handle");
    let cp = handle.cp.lock().await;
    use bee_control::control_plane::JobMode;
    assert_eq!(cp.job_mode(job_a), JobMode::Producer);
    assert_eq!(cp.job_mode(job_b), JobMode::Subscriber);
}
```

(The `Deployer::new(DeployerConfig::default()).await` constructor signature is what `deploy_pipeline.rs:27,49,70` uses today; the test imports it from there.)

- [ ] **Step 8: Run the deployer_s17 test suite; all 7 should PASS**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control --test deployer_s17 2>&1 | tail -15
```

Expected: `test result: ok. 7 passed; 0 failed`. (5 from Task 8 + 2 new end-to-end.)

- [ ] **Step 9: Run the full bee-dsl-sql and bee-control test suites**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-dsl-sql -p bee-control 2>&1 | tail -10
```

Expected: all green.

- [ ] **Step 10: Commit (GREEN)**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-dsl-sql/src/preprocess.rs crates/bee-dsl-sql/src/lib.rs crates/bee-control/src/deployer.rs crates/bee-control/tests/deployer_s17.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §2 GREEN: extract_stream_identities + Deployer wires Producer/Subscriber by signature"
```

---

## Task 10: §5 StreamSubscriber — add `bee-control` dep to `bee-runtime`

**Files:**
- Modify: `crates/bee-runtime/Cargo.toml`

- [ ] **Step 1: Add `bee-control` to `bee-runtime`'s dependencies**

Read the current `Cargo.toml`:
```bash
cat /Users/shaw/Developer/rust/bee/crates/bee-runtime/Cargo.toml
```

Add `bee-control = { workspace = true }` to the `[dependencies]` block (if not already present).

- [ ] **Step 2: Verify `cargo check` passes**

```bash
cd /Users/shaw/Developer/rust/bee && cargo check -p bee-runtime 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-runtime/Cargo.toml && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §5 setup: add bee-control dep to bee-runtime for JobLifecycleState"
```

---

## Task 11: §5 StreamSubscriber — write the failing state-machine test (RED)

**Files:**
- Create: `crates/bee-runtime/src/subscriber.rs` (with failing test module)

- [ ] **Step 1: Create the file with the test module**

```rust
// crates/bee-runtime/src/subscriber.rs

//! S17 §5: StreamSubscriber state machine.
//!
//! Watches the ControlPlane for the upstream Producer's lifecycle
//! and re-establishes the BRP subscription when the Producer comes
//! back. The state machine is pure: given (current state, upstream
//! lifecycle, upstream presence), it produces the next state + an
//! action the runtime should take. The watcher / BRP wire is
//! layered on top; for S17 the wire is mocked and left as a
//! follow-up issue.

use bee_control::kv::JobLifecycleState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriberState {
    Connecting,
    Active,
    WaitingForUpstream,
    Resubscribing,
}

pub enum SubscriberAction {
    None,
    OpenSubscription,
    CloseSubscription,
    ReopenSubscriptionFrom { from_offset: u64 },
}

pub struct SubscriberTick {
    pub next: SubscriberState,
    pub action: SubscriberAction,
}

pub struct StreamSubscriber {
    pub upstream_job: u32,
    pub stream_sig: String,
    pub last_consumed_offset: u64,
    pub state: SubscriberState,
}

impl StreamSubscriber {
    pub fn new(upstream_job: u32, stream_sig: String) -> Self {
        Self {
            upstream_job,
            stream_sig,
            last_consumed_offset: 0,
            state: SubscriberState::Connecting,
        }
    }

    /// Drive one state-machine tick. Pure function — no I/O.
    pub fn tick(
        &mut self,
        upstream_lifecycle: JobLifecycleState,
        upstream_present: bool,
    ) -> SubscriberTick {
        // RED placeholder: stays in Connecting forever. The real
        // transition matrix lands in Task 12.
        let _ = (upstream_lifecycle, upstream_present);
        SubscriberTick {
            next: SubscriberState::Connecting,
            action: SubscriberAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_control::kv::JobLifecycleState::*;

    fn sub() -> StreamSubscriber {
        StreamSubscriber::new(1, "sig".into())
    }

    #[test]
    fn connecting_to_active_when_upstream_running() {
        let mut s = sub();
        let t = s.tick(Running, true);
        assert_eq!(t.next, SubscriberState::Active);
        assert!(matches!(t.action, SubscriberAction::OpenSubscription));
    }

    #[test]
    fn connecting_stays_connecting_when_upstream_not_running() {
        let mut s = sub();
        let t = s.tick(Scheduled, true);
        assert_eq!(t.next, SubscriberState::Connecting);
        assert!(matches!(t.action, SubscriberAction::None));
    }

    #[test]
    fn active_to_waiting_when_upstream_fails() {
        let mut s = sub();
        s.tick(Running, true); // -> Active
        let t = s.tick(Failed, true);
        assert_eq!(t.next, SubscriberState::WaitingForUpstream);
        assert!(matches!(t.action, SubscriberAction::CloseSubscription));
    }

    #[test]
    fn active_to_waiting_when_upstream_disappears() {
        let mut s = sub();
        s.tick(Running, true); // -> Active
        let t = s.tick(Running, false);
        assert_eq!(t.next, SubscriberState::WaitingForUpstream);
        assert!(matches!(t.action, SubscriberAction::CloseSubscription));
    }

    #[test]
    fn waiting_to_resubscribing_when_upstream_running_again() {
        let mut s = sub();
        s.tick(Running, true);
        s.tick(Failed, true);
        let t = s.tick(Running, true);
        assert_eq!(t.next, SubscriberState::Resubscribing);
        assert!(matches!(t.action, SubscriberAction::ReopenSubscriptionFrom { .. }));
    }

    #[test]
    fn resubscribing_to_active_after_reopen() {
        let mut s = sub();
        s.last_consumed_offset = 42;
        s.tick(Running, true);
        s.tick(Failed, true);
        s.tick(Running, true); // -> Resubscribing
        let t = s.tick(Running, true); // re-subscribe complete
        assert_eq!(t.next, SubscriberState::Active);
        // ReopenSubscriptionFrom carries the last offset.
        s.last_consumed_offset = 100;
        let t = s.tick(Failed, true); // upstream dies mid-resub
        assert_eq!(t.next, SubscriberState::WaitingForUpstream);
    }

    #[test]
    fn waiting_stays_waiting_while_upstream_absent() {
        let mut s = sub();
        s.tick(Running, true);
        s.tick(Failed, true);
        let t = s.tick(Failed, false);
        assert_eq!(t.next, SubscriberState::WaitingForUpstream);
        assert!(matches!(t.action, SubscriberAction::None));
    }

    #[test]
    fn active_stays_active_on_lifecycle_pause() {
        // Upstream briefly flips to WaitingForUpstream (its own
        // dep resolution); this is not a death event.
        let mut s = sub();
        s.tick(Running, true);
        let t = s.tick(WaitingForUpstream, true);
        assert_eq!(t.next, SubscriberState::Active,
            "a transient upstream pause must not sever the subscription");
    }
}
```

- [ ] **Step 2: Re-export from `crates/bee-runtime/src/lib.rs`**

Add `pub mod subscriber;` to the module declarations (the file currently has `mod` blocks; read it first to find the right insertion point):

```rust
pub mod subscriber;
```

- [ ] **Step 3: Run the tests; they should all FAIL**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-runtime --lib subscriber:: 2>&1 | tail -25
```

Expected: 8 failed tests. The `connecting_to_active_when_upstream_running` test fails because the placeholder returns `Connecting`, not `Active`. The `active_*` tests fail because the state never advances past `Connecting`.

- [ ] **Step 4: Commit (RED)**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-runtime/src/subscriber.rs crates/bee-runtime/src/lib.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §5 RED: 8 StreamSubscriber state-machine tests (all failing — placeholder body)"
```

---

## Task 12: §5 StreamSubscriber — implement and pass the test (GREEN)

**Files:**
- Modify: `crates/bee-runtime/src/subscriber.rs` (only the `tick` body)

- [ ] **Step 1: Replace the `tick` body with the real transition matrix**

Replace the `tick` body (currently:
```rust
        // RED placeholder: stays in Connecting forever. The real
        // transition matrix lands in Task 12.
        let _ = (upstream_lifecycle, upstream_present);
        SubscriberTick {
            next: SubscriberState::Connecting,
            action: SubscriberAction::None,
        }
```
) with:

```rust
        use JobLifecycleState::*;
        use SubscriberAction::*;
        use SubscriberState::*;

        let (next, action) = match (
            self.state, upstream_present, upstream_lifecycle,
        ) {
            // Connecting
            (Connecting, true, Running) => (Active, OpenSubscription),
            (Connecting, _, _) => (Connecting, None),

            // Active
            (Active, true, Running) => (Active, None),
            (Active, true, WaitingForUpstream) => (Active, None),
            (Active, true, Failed | Completed) => (WaitingForUpstream, CloseSubscription),
            (Active, false, _) => (WaitingForUpstream, CloseSubscription),

            // WaitingForUpstream
            (WaitingForUpstream, true, Running) => (
                Resubscribing,
                ReopenSubscriptionFrom { from_offset: self.last_consumed_offset },
            ),
            (WaitingForUpstream, _, _) => (WaitingForUpstream, None),

            // Resubscribing
            (Resubscribing, true, Running) => (Active, None),
            (Resubscribing, _, _) => (WaitingForUpstream, CloseSubscription),
        };

        SubscriberTick { next, action }
```

- [ ] **Step 2: Run the tests; they should all PASS**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-runtime --lib subscriber:: 2>&1 | tail -15
```

Expected: `test result: ok. 8 passed; 0 failed`.

- [ ] **Step 3: Commit (GREEN)**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-runtime/src/subscriber.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §5 GREEN: StreamSubscriber state machine (Connecting|Active|Waiting|Resubscribing)"
```

---

## Task 13: §5 Cluster-level reconnect integration test

**Files:**
- Create: `crates/bee-runtime/tests/subscriber.rs`

- [ ] **Step 1: Create the integration test that drives the full CP**

```rust
// crates/bee-runtime/tests/subscriber.rs

use std::time::Duration;
use bee_control::control_plane::JobMode;
use bee_control::kv::{JobLifecycleState, Op};
use bee_control::raft::cluster::{Cluster, ClusterConfig};
use bee_runtime::subscriber::{StreamSubscriber, SubscriberState};

async fn fresh_cluster() -> Cluster {
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster.wait_for_leader(Duration::from_secs(5)).await.expect("leader");
    cluster
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscriber_state_machine_drives_off_real_cp_lifecycle() {
    let cluster = fresh_cluster().await;
    let leader = cluster.leader().await.expect("leader");

    // Setup: Job A is a Producer; Job B is a Subscriber with a
    // dependency on A.
    cluster.submit(leader, Op::RegisterDatasourceProducer { signature: "sig".into(), job_id: 1 })
        .await.expect("");
    cluster.submit(leader, Op::RegisterJob { job_id: 2, dag_hash: "d".into(), owner_node: leader, tenant: 0 })
        .await.expect("");
    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 2, state: JobLifecycleState::Running })
        .await.expect("");
    cluster.submit(leader, Op::RegisterDependency { downstream_job: 2, upstream_job: 1, stream: "sig".into() })
        .await.expect("");

    // The Subscriber starts in Connecting; once A is Running, the
    // state machine drives it to Active.
    let mut sub = StreamSubscriber::new(1, "sig".into());
    assert_eq!(sub.state, SubscriberState::Connecting);

    // Simulate a tick: A is Running and present.
    let lifecycle = {
        let handle = cluster.handle(leader).expect("handle");
        let cp = handle.cp.lock().await;
        cp.list_jobs().iter().find(|j| j.job_id == 1).expect("A exists").lifecycle
    };
    let t = sub.tick(lifecycle, true);
    assert_eq!(t.next, SubscriberState::Active);

    // Now A dies. The state machine should drive the Subscriber
    // to WaitingForUpstream.
    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 1, state: JobLifecycleState::Failed })
        .await.expect("");
    // And the orchestrator-side propagation flips B to
    // WaitingForUpstream (not exercised here, but the upstream
    // lifecycle that the SM sees is Failed).
    let lifecycle = {
        let handle = cluster.handle(leader).expect("handle");
        let cp = handle.cp.lock().await;
        cp.list_jobs().iter().find(|j| j.job_id == 1).expect("A exists").lifecycle
    };
    let t = sub.tick(lifecycle, true);
    assert_eq!(t.next, SubscriberState::WaitingForUpstream);

    // A comes back. The SM drives the Subscriber through
    // Resubscribing back to Active.
    cluster.submit(leader, Op::UpdateJobLifecycle { job_id: 1, state: JobLifecycleState::Running })
        .await.expect("");
    let lifecycle = {
        let handle = cluster.handle(leader).expect("handle");
        let cp = handle.cp.lock().await;
        cp.list_jobs().iter().find(|j| j.job_id == 1).expect("A exists").lifecycle
    };
    let t = sub.tick(lifecycle, true);
    assert_eq!(t.next, SubscriberState::Resubscribing);
    let t = sub.tick(lifecycle, true);
    assert_eq!(t.next, SubscriberState::Active);

    // Sanity: the Mode column is consistent (B remains Subscriber
    // through the whole dance; A is Producer when running).
    let handle = cluster.handle(leader).expect("handle");
    let cp = handle.cp.lock().await;
    assert_eq!(cp.job_mode(1), JobMode::Producer);
    assert_eq!(cp.job_mode(2), JobMode::Subscriber);
}
```

- [ ] **Step 2: Run the test**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-runtime --test subscriber 2>&1 | tail -15
```

Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Step 3: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-runtime/tests/subscriber.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17 §5: Cluster-level integration test for StreamSubscriber reconnect cycle"
```

---

## Task 14: Final — full workspace check + consolidate into single S17 commit

**Files:**
- (git history)

- [ ] **Step 1: Run the full workspace test suite**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | tail -10
```

Expected: all green. (Plugins/ crates have no tests, so they may not contribute to the count; that's fine.)

- [ ] **Step 2: Run `cargo build --workspace` to confirm 0 warnings**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build --workspace 2>&1 | tail -20
```

Expected: `Finished` line, no warnings. Pre-existing warnings in `bee-control/tests/deploy_pipeline.rs:75` and `crates/bee-control/tests/raft_cluster.rs:45` (unused variable / unused mut) are out of scope for S17; do not fix them in this commit.

- [ ] **Step 3: Look at the commit history since the design commit**

```bash
cd /Users/shaw/Developer/rust/bee && git log --oneline 35c964e..HEAD
```

You should see roughly 10–13 commits: §0 (deps), §1 RED, §1 GREEN, §3 RED, §3 GREEN, §4 RED, §4 GREEN, §2 (CP-level), §2 GREEN, §5 setup, §5 RED, §5 GREEN, §5 integration.

- [ ] **Step 4: Consolidate into a single `S17:` commit**

```bash
cd /Users/shaw/Developer/rust/bee && git reset --soft 35c964e && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S17: StreamSignature (sha256) + Producer/Subscriber detection + reconnect

- signature::stream_signature: canonical sha256 of
  (datasource_name, adapter_method, stream_topology_args) per ADR-0011
- deployer: computes signatures, registers Producer only for new
  sigs; same-sig deploys become Subscribers with dependency on
  existing Producer
- control_plane: Op::RegisterDatasourceProducer (existed; reused) +
  propagate_producer_death (Producer Failed/Completed/removed ->
  all Subscribers -> WaitingForUpstream)
- jobs_view: JobMode enum + Mode column
  (Producer | Subscriber | Independent), derived at view time, no
  JobRecord field change
- dsl_sql::preprocess::extract_stream_identities: extract
  (datasource, method, args) from 'use X; X.method(args)' SQL
- runtime::StreamSubscriber: state machine
  (Connecting | Active | WaitingForUpstream | Resubscribing);
  BRP re-subscribe wire marked todo!() with follow-up issue

Acceptance (all green):
- Same-sig deploys result in 1 Producer + N Subscribers
- Different sigs get different Producers
- Producer death -> Subscribers flip to WaitingForUpstream (CP)
- StreamSubscriber full cycle:
  Connecting -> Active -> WaitingForUpstream -> Resubscribing -> Active
- bee jobs list shows Mode column
- No regression: existing S16 / S18 / S29 tests still pass

7 new test files / test groups. Single-commit deliverable per
docs/superpowers/specs/2026-06-07-s17-stream-signature-design.md
§7. Working tree (S33 wrap-up, ADR-0011, plugins/ scaffolds) is
left untouched per the approved scope."
```

- [ ] **Step 5: Verify the single commit**

```bash
cd /Users/shaw/Developer/rust/bee && git log --oneline 35c964e..HEAD
```

Expected: a single commit, message starts with `S17: StreamSignature ...`.

- [ ] **Step 6: Verify the full workspace is still green from the new HEAD**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | tail -5
```

Expected: all green.

- [ ] **Step 7: Check the working tree — S33 wrap-up should be untouched**

```bash
cd /Users/shaw/Developer/rust/bee && git status --short
```

Expected:
```
 M Cargo.toml
 M README.md
 M docs/adr/README.md
 M docs/product-design.md
 M docs/stories.md
?? docs/adr/0011-stream-identity-and-backfill.md
?? plugins/
```

The S33 wrap-up is still uncommitted (untouched by this plan), and the `docs/superpowers/specs/2026-06-07-s17-stream-signature-design.md` and S17 commit are now on `main`.

---

## Self-review checklist (run before claiming done)

- [ ] **Spec coverage:** §1 ✓ (Tasks 2–3), §2 ✓ (Tasks 8–9), §3 ✓ (Tasks 4–5), §4 ✓ (Tasks 6–7), §5 ✓ (Tasks 10–13).
- [ ] **No placeholders:** all 6 test bodies, all 4 production code blocks, the `extract_stream_identities` parser, the deployer wiring, the `StreamSubscriber::tick` matrix are spelled out.
- [ ] **Type consistency:**
  - `stream_signature(&str, &str, &BTreeMap<String, String>) -> String` — used the same way in §1, §2, and the deployer wiring.
  - `JobMode` enum is `pub` in `control_plane`, re-imported as `bee_control::control_plane::JobMode` in tests.
  - `propagate_producer_death(u32) -> Vec<u32>` — same signature in declaration, in `control_plane.rs`, and in `producer_subscriber.rs` tests.
  - `StreamSubscriber::tick(JobLifecycleState, bool) -> SubscriberTick` — same in mod tests and integration test.
  - `Pipeline::stream_identities()` → `extract_stream_identities(&str) -> Vec<(String, String, BTreeMap<String, String>)>` — used in deployer step 5.
- [ ] **DRY:** the `Cluster` setup helper is duplicated in 3 test files (`producer_subscriber.rs`, `jobs_view.rs`, `deployer_s17.rs`); that's an acceptable test-level DRY violation in Rust (test helpers are intentionally inlined per test file to keep the harness simple).
- [ ] **Frequent commits:** 12 small commits before consolidation; consolidated to 1 in Task 14.
- [ ] **YAGNI:** no extra features beyond the spec; `JobMode::Independent` is added because the spec's "show me which is which" column demands a default; the BRP wire is explicitly `todo!()`-marked.
- [ ] **TDD discipline:** every production code change is preceded by a failing test in a separate commit. The `extract_stream_identities` parser tests in Task 9 Step 3 are best-effort — if the parser's exact shape is hard to test deterministically, the priority is the `deployer_s17.rs` end-to-end test (Step 6) which gives us coverage of the user-visible contract.

## Out-of-scope items (do not address in this plan)

- BRP re-subscribe wire protocol (the actual TCP/Protobuf handshake). State machine + tests land; the wire is `todo!()` + follow-up issue. Not a blocker for S17 commit.
- `JobRecord.mode` field. Mode is derived at view time only; no schema change.
- The S33 working tree (mock plugin scaffolds, ADR-0011, stories.md S33-S41, etc.). Untouched.
- Datasource-level signature (Provider identity). Out of scope per the user's choice; ADR-0010's `Datasource.plugin_id` already serves the Provider-identity role.
- Orchestrator-side auto-call to `propagate_producer_death`. The function exists and is tested; the orchestrator wiring is a one-liner that lives in a follow-up commit.

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-07-s17-stream-signature.md`. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** — execute tasks in this session using `executing-plans`, batch execution with checkpoints

Which approach?
