# S17 · StreamSignature (sha256) + Producer/Subscriber detection + reconnect on Producer restart

**Date**: 2026-06-07
**Status**: design — pending approval
**Owner**: S17
**Supersedes / refines**: ADR-0003 §"StreamSignature" definition; ADR-0010 §"per-call args"
**Blocked by**: S16 (Datasource Adapter trait + test fixture), S18 (cross-Pipeline dependency tracking — used as the Subscriber dependency mechanism)
**Story**: `docs/stories.md` §S17 (lines 483–503)

## Scope (this session)

End-to-end implementation of S17:

1. `StreamSignature` computation: canonical `sha256(datasource_name || adapter_method || sha256(stream_topology_args))` per ADR-0011.
2. Deployer wiring: `Deployer::deploy` computes the signature per stream-producing Phase, registers a Producer only when no Producer exists for the signature; otherwise adds a `DependencyRecord` so the Job becomes a Subscriber of the existing Producer.
3. CP propagation: when a Producer Job's `JobLifecycleState` becomes `Failed` / `Completed` / is removed, all Subscribers' lifecycle flips to `WaitingForUpstream`.
4. `bee jobs list` Mode column: derived from the CP `datasource_producers` registry + each Job's `dependencies` (`Producer | Subscriber | Independent`).
5. Runtime re-subscribe: a `StreamSubscriber` state machine (`Connecting → Active → WaitingForUpstream → Resubscribing → Active`) that watches the CP for upstream liveness and re-establishes the subscription when the Producer comes back.
6. Tests (TDD): 6 new test files covering hash determinism, signature-based idempotency, propagation, deployer end-to-end, view rendering, and subscriber state transitions.

## Out of scope (deferred)

- The actual BRP re-subscribe wire protocol (the runtime state machine + unit tests land; the BRP data-channel handshake is marked `todo!()` with a follow-up issue; not a blocker for S17 commit).
- S18 cross-Pipeline edges' "in-process" optimization is reused as-is; no new "in-process stream subscription" code path.
- `DatasourceSignature` (Provider-level hash) is **not** implemented; ADR-0010's `Datasource` struct already carries `plugin_id` which serves as the Provider identity. Provider-level dedup is out of scope for S17.
- The S33 working tree (mock plugin scaffolds, ADR-0011, S33–S41 story additions) is **not** committed or modified in this session. S17 stands alone.
- `JobRecord.mode` field is **not** added; Mode is derived at view time only.

## Architecture

### §1. Hash function — `crates/bee-control/src/signature.rs` (new)

```rust
use std::collections::BTreeMap;
use sha2::{Digest, Sha256};

/// ADR-0011 §1: a Stream's identity is the hash of its **stream
/// topology**, not its per-call resumption parameters. Two Pipelines
/// that share a topology share a Producer.
pub fn stream_signature(
    datasource_name: &str,
    adapter_method: &str,
    stream_topology_args: &BTreeMap<String, String>,
) -> String {
    // inner: sha256 over the canonical JSON of the topology args.
    // BTreeMap serializes in key-sorted order -> canonical for free.
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
        assert_eq!(a, b);
    }

    #[test]
    fn different_datasource_yields_different_signature() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT")]));
        let b = stream_signature("google_news", "search",
            &args(&[("query", "btc")]));
        assert_ne!(a, b);
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
    }

    #[test]
    fn empty_args_yields_valid_signature() {
        let s = stream_signature("binance", "ping",
            &BTreeMap::new());
        // 64 hex chars = 32 bytes = sha256 output
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
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

**`Cargo.toml` additions** to `crates/bee-control/Cargo.toml`:
```toml
sha2 = "0.10"
hex = "0.4"
serde_json = "1"
```

(`serde_json` is already used transitively via `bee-dsl-sql`; declaring it explicitly here is the cleaner boundary.)

### §2. Deployer wiring — `crates/bee-control/src/deployer.rs`

`Deployer::deploy` (currently a stub at `deployer.rs:184`) gains the following pre-`RegisterJob` step:

```rust
let stream_sigs = pipeline.stream_identities(); // Vec<StreamIdentity>
//   StreamIdentity = (datasource_name, adapter_method, topology_args)
let mut producer_sigs: Vec<String> = vec![];
let mut sub_deps: Vec<DependencyRecord> = vec![];
for ident in &stream_sigs {
    let sig = stream_signature(&ident.0, &ident.1, &ident.2);
    if let Some(producer_id) = cp.lookup_datasource_producer(&sig).await {
        sub_deps.push(DependencyRecord {
            upstream_job: producer_id,
            stream: sig,
        });
    } else {
        producer_sigs.push(sig);
    }
}
// submit RegisterJob first (so we have job_id), then the deps and
// RegisterDatasourceProducer ops in the same Raft batch when possible
```

**`Pipeline::stream_identities()`**: new method on `Pipeline` in `crates/bee-dsl-sql/src/...`. Walks the parsed AST (`UseDirective` nodes joined with the next `Call` AST node) and returns `Vec<(datasource_name, adapter_method, BTreeMap<String, String>)>`. The exact AST shape is discovered during implementation by reading `preprocess.rs` lines 270–300 (which already has `(datasource_name, adapter, config)` joins for the `use <name>;` path).

**Tests** (in `crates/bee-control/tests/deployer_s17.rs`, new):

| # | Scenario | Expected |
| --- | --- | --- |
| 1 | Deploy Pipeline A `binance.subscribe('BTC/USDT','5min')` | `JobId(A)`; one entry in `datasource_producers`; no Subscriber deps |
| 2 | Deploy Pipeline B (same call) | `JobId(B)`; `datasource_producers` still has 1 entry; `B.dependencies` contains `{upstream_job: A, stream: <sig>}` |
| 3 | Deploy Pipeline C `binance.subscribe('ETH/USDT','5min')` | `JobId(C)`; `datasource_producers` has 2 entries; C has no Subscriber dep |
| 4 | Deploy Pipeline D `binance.subscribe('BTC/USDT','1min')` | `JobId(D)`; `datasource_producers` has 3 entries (interval is topology) |
| 5 | Re-deploy A (same sig) | `JobId(A)` reused; idempotent; A remains the Producer |

### §3. CP propagation — `crates/bee-control/src/control_plane.rs`

```rust
/// S17: when a Producer Job dies, all Subscribers must flip to
/// `WaitingForUpstream`. Returns the list of JobIds that were
/// flipped (for the orchestrator / log). Idempotent.
pub fn propagate_producer_death(
    &mut self,
    producer_job_id: u32,
) -> Vec<u32> {
    let mut flipped = vec![];
    for (job_id, job) in self.jobs.iter_mut() {
        let depends_on_dead = job.dependencies
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

Called by the orchestrator / a periodic tick after `Op::UpdateJobLifecycle { state: Failed }` or `Op::UpdateJobLifecycle { state: Completed }` is applied to a Job that is registered as a Producer. The orchestrator code change is out of scope for S17 — the function exists and is tested; the caller is a one-liner the orchestrator can add in a follow-up.

**Tests** (in `crates/bee-control/tests/producer_subscriber.rs`, extending the existing 3 tests):

| # | Scenario | Expected |
| --- | --- | --- |
| 4 | Register Job A as Producer; Register Job B with `upstream_job = A`; B is `Running`; call `propagate_producer_death(A)` | B's lifecycle becomes `WaitingForUpstream`; return value is `[B]` |
| 5 | Same as 4, but B is already `WaitingForUpstream` | No flip; return value is `[]` |
| 6 | Producer A dies, then B is registered as a new Subscriber of A's slot | B starts as `WaitingForUpstream` (no special case needed; standard dep-resolved lifecycle) |

### §4. `bee jobs list` Mode column — `crates/bee-control/src/jobs_view.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMode {
    Producer,     // this job_id is in datasource_producers.values()
    Subscriber,   // this job has a dep whose upstream is a Producer
    Independent,  // neither
}

impl ControlPlaneStateMachine {
    pub fn job_mode(&self, job_id: u32) -> JobMode {
        if self.datasource_producers.values().any(|&p| p == job_id) {
            return JobMode::Producer;
        }
        if let Some(j) = self.jobs.get(&job_id) {
            for d in &j.dependencies {
                if self.datasource_producers.values().any(|&p| p == d.upstream_job) {
                    return JobMode::Subscriber;
                }
            }
        }
        JobMode::Independent
    }
}
```

`jobs_view` gains a `MODE` column rendered as `Producer` / `Subscriber` / `-` (for Independent).

**Tests** (in `crates/bee-control/tests/jobs_view.rs`, extending the existing tests):

| # | Scenario | Expected Mode |
| --- | --- | --- |
| 1 | Job A registered as Producer; B registered as Subscriber (via dep) | A=Producer, B=Subscriber |
| 2 | Job C has no signature, no dep | C=Independent |
| 3 | Job D is a Producer AND has a Subscriber dep (chained: D produces X, D also subscribes to Y) | D=Producer (Producer wins) |

### §5. Runtime re-subscribe — `crates/bee-runtime/src/subscriber.rs` (new)

```rust
//! S17 §5: StreamSubscriber state machine.
//!
//! Watches the CP for the upstream Producer's lifecycle and
//! re-establishes the subscription when the Producer comes back.
//! On restart, the Subscriber resumes from its last consumed
//! offset (stored in Task State per ADR-0004).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriberState {
    /// Initial state; opening the BRP subscription.
    Connecting,
    /// Live: receiving events from upstream.
    Active,
    /// Upstream is `Failed`/`Completed`/missing; waiting.
    WaitingForUpstream,
    /// Upstream is `Running` again; re-opening the BRP subscription.
    Resubscribing,
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

    /// Drive one state-machine tick. Pure function over (self, upstream
    /// lifecycle) so it is unit-testable without a real cluster.
    pub fn tick(
        &mut self,
        upstream_lifecycle: JobLifecycleState,
        upstream_present: bool,
    ) -> SubscriberTick {
        use JobLifecycleState::*;
        use SubscriberState::*;
        let next = match (self.state, upstream_present, upstream_lifecycle) {
            (Connecting, true, Running) => Active,
            (Connecting, _, _) => Connecting, // still waiting for upstream to come up
            (Active, true, Running) => Active,
            (Active, true, WaitingForUpstream) => Active, // upstream alive, just paused
            (Active, false, _) | (Active, true, Failed | Completed) => WaitingForUpstream,
            (WaitingForUpstream, true, Running) => Resubscribing,
            (WaitingForUpstream, _, _) => WaitingForUpstream,
            (Resubscribing, true, Running) => Active, // re-subscribe complete
            (Resubscribing, _, _) => WaitingForUpstream, // upstream died mid-resub
        };
        let action = SubscriberAction::from_transition(self.state, next);
        self.state = next;
        SubscriberTick { next, action }
    }
}

pub struct SubscriberTick {
    pub next: SubscriberState,
    pub action: SubscriberAction,
}

pub enum SubscriberAction {
    None,
    OpenSubscription,
    CloseSubscription,
    ReopenSubscriptionFrom { from_offset: u64 },
}
```

**Wire-up to BRP re-subscribe**: the actual `ReopenSubscriptionFrom { from_offset }` action is where the runtime re-establishes the BRP data channel. The current `bee-transport` does not have a `Resubscribe` RPC. For S17 the action is logged and a `todo!()`-marked TODO comment is left; the integration test mocks this out. The follow-up issue tracks the BRP wire protocol.

**Tests**:

- `crates/bee-runtime/src/subscriber.rs` (mod tests): 6+ unit tests covering the full transition matrix.
- `crates/bee-runtime/tests/subscriber.rs` (new): integration test using `Cluster` to drive a real CP — verifies that after `propagate_producer_death(A)`, a `StreamSubscriber` watching A transitions `Active → WaitingForUpstream → Resubscribing → Active` when A's lifecycle is restored to `Running`.

### §6. Test inventory (TDD order)

| # | File | Kind | Covers |
| --- | --- | --- | --- |
| 1 | `crates/bee-control/src/signature.rs` mod tests | unit | §1 hash determinism + sensitivity |
| 2 | `crates/bee-control/tests/signature_integration.rs` | integration (Cluster) | §1 hash → `RegisterDatasourceProducer` idempotency end-to-end |
| 3 | `crates/bee-control/tests/producer_subscriber.rs` (extend) | integration (Cluster) | §3 propagation: 3 new test cases |
| 4 | `crates/bee-control/tests/deployer_s17.rs` (new) | integration (Cluster) | §2 deployer: 5 test cases |
| 5 | `crates/bee-control/tests/jobs_view.rs` (extend) | integration (Cluster) | §4 mode column: 3 new test cases |
| 6 | `crates/bee-runtime/src/subscriber.rs` mod tests | unit | §5 state machine matrix |
| 7 | `crates/bee-runtime/tests/subscriber.rs` (new) | integration (Cluster) | §5 full reconnect path |

### §7. Dependency changes

```toml
# crates/bee-control/Cargo.toml
[dependencies]
sha2 = "0.10"        # NEW
hex = "0.4"          # NEW
serde_json = "1"     # NEW (was transitive)
# (existing deps unchanged)

# crates/bee-runtime/Cargo.toml
[dependencies]
# may need to add `tokio` if not present, but it almost certainly is
```

## Acceptance criteria

- [ ] `cargo build --workspace` clean (0 warnings)
- [ ] `cargo test --workspace` all pass; 7 new test files / test groups
- [ ] `StreamSignature` matches ADR-0011 formula exactly
- [ ] Deploying two Pipelines with the same `use exchange; exchange.subscribe(symbol='X/Y', interval='5min')` produces one Producer + one Subscriber (asserted via `datasource_producers` registry)
- [ ] Deploying two Pipelines with different `symbol` produces two Producers
- [ ] Killing a Producer's Node transitions all Subscribers to `WaitingForUpstream` (CP-level)
- [ ] `bee jobs list` shows `Producer` / `Subscriber` / `Independent` mode column
- [ ] `StreamSubscriber` state machine: full `Connecting → Active → WaitingForUpstream → Resubscribing → Active` cycle passes
- [ ] No regression: existing S16 / S18 / S29 tests still pass

## Risks

1. **`Pipeline::stream_identities()` AST shape**: not yet read in detail. The exact join between `UseDirective` and the next `Call` AST node may need a small refactor in `preprocess.rs`. **Mitigation**: this is the first thing to investigate in step 1 of implementation; if the AST doesn't carry `(method, args)` directly, the extract step may need a 10–30 line helper rather than a 1-liner.
2. **Cross-Node BRP re-subscribe wire**: the `ReopenSubscriptionFrom` action has no protocol yet. **Mitigation**: state machine + unit tests ship in S17; integration test mocks the BRP handshake; follow-up issue is filed. The CP-level state machine is the binding contract; the BRP wire is implementation detail.
3. **`cargo check` warning churn**: adding `sha2`/`hex` to `bee-control` may surface new lints in dependent crates that already use them transitively. **Mitigation**: `cargo build --workspace` is the gate; any new warnings get fixed in the same commit.
4. **Worker-thread test flakiness**: `Cluster` tests use `worker_threads = 2`; the new integration tests should keep the same pattern.

## Implementation order

1. **§1** (RED) Write `signature.rs` mod tests first; run, see them fail (no `stream_signature` exists).
2. **§1** (GREEN) Implement `stream_signature`; run, see them pass.
3. **§3** (RED) Extend `producer_subscriber.rs` with the 3 propagation tests; run, see them fail (no `propagate_producer_death` exists).
4. **§3** (GREEN) Implement `propagate_producer_death`; run, see them pass.
5. **§2** (RED) Read `preprocess.rs` to understand the AST shape; write `Pipeline::stream_identities()` stub; write the 5 deployer tests; run, see them fail.
6. **§2** (GREEN) Implement `Pipeline::stream_identities()` and the deployer wiring; run, see them pass.
7. **§4** (RED) Write the 3 jobs_view tests; run, see them fail.
8. **§4** (GREEN) Implement `JobMode` + `job_mode()`; run, see them pass.
9. **§5** (RED) Write `StreamSubscriber` mod tests; run, see them fail.
10. **§5** (GREEN) Implement `StreamSubscriber`; run, see them pass.
11. **§5** (RED→GREEN) Write `subscriber.rs` integration test using `Cluster`; mock BRP; pass.
12. **Final**: `cargo build --workspace` clean, `cargo test --workspace` all green.
13. **Commit**: single `S17: ...` commit per §7 of this design.

## Open questions (resolved during design)

- ❓ Two signatures or one? **Resolved: one (StreamSignature only).**
- ❓ Reconnect scope? **Resolved: CP + runtime state machine; BRP wire deferred.**
- ❓ Commit strategy? **Resolved: single commit (consistent with S22/S23/S31/S33).**
- ❓ JobRecord.mode field or derived? **Resolved: derived at view time only.**
