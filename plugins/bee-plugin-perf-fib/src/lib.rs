//! `bee-plugin-perf-fib` — Fibonacci Handler UDFs for the S41
//! performance showcase.
//!
//! Two Handler UDFs are declared:
//! - [`fib_seed`]: stateless; returns 0 (n = 0) or 1 (n >= 1).
//! - [`fib_step`]: stateful; reads its own previous 2 emitted values
//!   from KV-backed Task State and writes the new value back
//!   (full implementation lands in Task 6).
//!
//! This file is the **scaffold** (Task 4). The `fib_step` UDF body
//! is a placeholder that returns 0; Task 6 will replace it with the
//! KV-backed implementation that uses `BeeHostV1::safe_kv_get` /
//! `safe_kv_put` (added in Tasks 1–3) plus the [`FibState`] type
//! defined here.
//!
//! ## Architecture
//!
//! - [`FibState`]: the two-value rolling state stored in KV (defined
//!   now so the Task 6 implementation has a stable shape to fill in).
//! - [`fib_seed`] / [`fib_step`]: pure compute functions. The
//!   runtime dispatches by handler name (declared in the manifest).
//! - [`PerfFibFactory`]: produces the `PluginManifest` +
//!   `PluginHandle` for the host.
//! - `cdylib_plugin!(PerfFibFactory)` at the bottom generates the
//!   FFI entry symbols (`bee_plugin_init` / `bee_plugin_drop`).

use std::sync::Arc;

use bee_plugin_sdk::{
    BeeHostV1, Factory, HandlerDescriptor, PluginHandle, PluginManifest,
    PluginName, SdkError,
};
use serde::{Deserialize, Serialize};

/// KV-stored state for [`fib_step`].
///
/// The Host persists one `FibState` per Task under a key derived
/// from the `TaskId` (and the `PluginId`, so old plugin state
/// survives a swap). On each invocation, `fib_step` reads the
/// previous state, computes the next Fibonacci value, and writes
/// the rolled state back.
///
/// The shape `(prev2, prev1)` (not `(last_n_minus_1, last_n_minus_2)`)
/// is the "two most recent emitted values" — same as
/// `FibState::next()` reads them in the order they were last
/// emitted. `update(new_value)` shifts: `prev2 <- prev1; prev1 <- new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FibState {
    pub prev2: i128,
    pub prev1: i128,
}

impl FibState {
    /// The next Fibonacci value = `prev2 + prev1`.
    pub fn next(&self) -> i128 {
        self.prev2 + self.prev1
    }

    /// Roll the state forward by `new_value`: `prev2 <- prev1`,
    /// `prev1 <- new_value`.
    pub fn update(&mut self, new_value: i128) {
        self.prev2 = self.prev1;
        self.prev1 = new_value;
    }
}

/// `fib_seed(n)`: returns 0 if `n == 0`, else 1.
///
/// This is the seed for the Fibonacci sequence: it produces the
/// first emitted value for a Task. The runtime is expected to
/// invoke `fib_seed(0)` exactly once per Task, then call
/// `fib_step(n)` for n = 1, 2, 3, … to continue the sequence.
pub fn fib_seed(n: u64) -> i128 {
    if n == 0 { 0 } else { 1 }
}

/// Compute the KV key for the [`fib_step`] state, scoped by the current
/// `stream_id` returned by the host. The key is the same for every call
/// inside one SQL query (the `stream_id` is the hash of the call site), so
/// the state evolves across calls within a Task.
fn state_key(host: &BeeHostV1) -> Result<String, SdkError> {
    let stream_id = host.safe_current_stream_id()?;
    Ok(format!(
        "state/handler/{}/fib_step/state",
        hex::encode(stream_id)
    ))
}

/// `fib_step(n)`: stateful Fibonacci step (Task 6, S41).
///
/// Reads the previous two emitted values from the host's KV, computes
/// `prev2 + prev1`, writes the rolled state back, and returns the new
/// value. If no state exists yet (first call, or fresh Task), the state
/// is treated as the Fibonacci seed pair `(0, 1)`, so the first emitted
/// value is `1`. A corrupted state is also treated as the seed pair —
/// this is intentionally lenient: a Task that restarts mid-sequence
/// simply resumes from the seed rather than panicking.
pub fn fib_step(host: &BeeHostV1, _n: u64) -> i128 {
    let key = state_key(host).expect("compute fib_step state key");
    let current = match host.safe_kv_get(&key) {
        Ok(Some(bytes)) => bincode::deserialize::<FibState>(&bytes)
            .unwrap_or(FibState { prev2: 0, prev1: 1 }),
        Ok(None) => FibState { prev2: 0, prev1: 1 },
        Err(_) => FibState { prev2: 0, prev1: 1 },
    };
    let new_value = current.next();
    let updated = FibState {
        prev2: current.prev1,
        prev1: new_value,
    };
    let bytes = bincode::serialize(&updated).unwrap();
    host.safe_kv_put(&key, &bytes).expect("kv_put fib_step state");
    new_value
}

/// The plugin's manifest. Declares 2 Handlers, no Adapters.
///
/// `feature_version` follows SemVer for human consumption;
/// `abi_version` is the binding contract the host checks at
/// load time (S20).
pub fn plugin_manifest() -> PluginManifest {
    PluginManifest {
        name: PluginName("bee-plugin-perf-fib".into()),
        feature_version: "0.1.0".into(),
        abi_version: "v1".into(),
        adapters: vec![],
        handlers: vec![
            HandlerDescriptor { name: "fib_seed".into() },
            HandlerDescriptor { name: "fib_step".into() },
        ],
    }
}

/// The plugin's factory. Holds no state itself: the per-Task
/// state lives in KV (see [`FibState`]).
pub struct PerfFibFactory;

impl Factory for PerfFibFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: Arc::new(()),
            input_adapters: std::collections::HashMap::new(),
            output_adapters: std::collections::HashMap::new(),
            handlers: std::collections::HashMap::new(),
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(PerfFibFactory);
