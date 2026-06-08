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
    Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName,
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

/// `fib_step(n)` (Task 4 placeholder).
///
/// The full Task 6 implementation will:
/// 1. Build the KV key from `current_stream_id()` + `n`.
/// 2. Read the prior `FibState` via `kv_get` (treat missing as
///    `FibState { prev2: 0, prev1: 1 }` — i.e. the seed values).
/// 3. Compute `state.next()` and `state.update(next)`.
/// 4. Write the rolled state back via `kv_put` and return the
///    computed value.
///
/// For the scaffold (Task 4), it returns 0 — Task 6 replaces this.
pub fn fib_step(n: u64) -> i128 {
    let _ = n;
    0
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
