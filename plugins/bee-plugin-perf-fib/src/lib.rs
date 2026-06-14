//! `bee-plugin-perf-fib` — Fibonacci Handler UDFs for the S41
//! performance showcase.
//!
//! S33.6.1: refactored to use the
//! `#[bee_adapter]` macro. The hand-written
//! FFI shims (`seed_shim`, `step_shim`)
//! + `write_event_bytes` / `decode_or_default`
//! helpers are gone — the macro generates
//! them.
//!
//! Two Handler UDFs are declared:
//! - [`fib_seed`]: stateless; returns 0 (n = 0) or 1 (n >= 1).
//! - [`fib_step`]: stateful; takes the previous two emitted values
//!   from a state blob and returns the next Fibonacci value,
//!   rolling the state forward.

use std::sync::Arc;

use bee_adapter::{AdapterError, AdapterResult};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::{
    Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName,
};
use serde::{Deserialize, Serialize};

/// Rolling state for [`fib_step`].
///
/// The host passes one `FibState` blob per invocation (the blob
/// from the previous call's `new_state_out`). On the first call
/// the host passes an empty blob; the plugin initializes the
/// state to the seed pair `(0, 1)`.
///
/// `(prev2, prev1)` is "the two most recent emitted values":
/// `next()` sums them, `update(new_value)` shifts the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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

/// `fib_seed` Handler (stateless). Returns
/// the seed Fibonacci value for `n`.
pub struct FibSeedHandler;

#[bee_adapter(handler, name = "fib_seed")]
impl FibSeedHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<FibState> {
        // Sentinel: stateless handler.
        Ok(FibState { prev2: 0, prev1: 0 })
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        _state: FibState,
        n: u64,
    ) -> AdapterResult<(FibState, i128)> {
        let value = if n == 0 { 0 } else { 1 };
        // Stateless: re-write the same sentinel.
        Ok((FibState { prev2: 0, prev1: 0 }, value))
    }
}

/// `fib_step` Handler (stateful). Rolls
/// the state forward and emits the new
/// Fibonacci value.
pub struct FibStepHandler;

#[bee_adapter(handler, name = "fib_step")]
impl FibStepHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<FibState> {
        // Seed the window as (0, 1) so the
        // first call's `prev2 + prev1 = 0 + 1
        // = 1` (the canonical Fibonacci
        // first value).
        Ok(FibState { prev2: 0, prev1: 1 })
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        state: FibState,
        n: u64,
    ) -> AdapterResult<(FibState, i128)> {
        // Lenient: a corrupt (zero) state
        // is treated as the seed.
        let current = if state.prev1 == 0 && state.prev2 == 0 {
            FibState { prev2: 0, prev1: 1 }
        } else {
            state
        };
        let new_value = current.next();
        let updated = FibState {
            prev2: current.prev1,
            prev1: new_value,
        };
        Ok((updated, new_value))
    }
}

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

pub struct PerfFibFactory;

impl Factory for PerfFibFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        let mut input_adapters = std::collections::HashMap::new();
        let mut output_adapters = std::collections::HashMap::new();
        let mut handlers = std::collections::HashMap::new();
        bee_plugin_sdk::register_vtable! {
            input_adapters, output_adapters, handlers;
            handler "fib_seed" => &FIB_SEED_HANDLER_VTABLE,
            handler "fib_step" => &FIB_STEP_HANDLER_VTABLE,
        }
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: Arc::new(()),
            input_adapters,
            output_adapters,
            handlers,
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(PerfFibFactory);

// ---- Pure-function re-exports for the in-process unit tests ----

pub fn fib_seed(n: u64) -> i128 {
    if n == 0 { 0 } else { 1 }
}

pub fn fib_step(state: FibState, n: u64) -> (FibState, i128) {
    let current = if state.prev1 == 0 && state.prev2 == 0 {
        FibState { prev2: 0, prev1: 1 }
    } else {
        state
    };
    let new_value = current.next();
    let updated = FibState {
        prev2: current.prev1,
        prev1: new_value,
    };
    (updated, new_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fib_step_pure_function_matches_known_values() {
        let s0 = FibState { prev2: 0, prev1: 0 };
        let (s1, v1) = fib_step(s0, 0);
        assert_eq!(v1, 1);
        assert_eq!(s1, FibState { prev2: 1, prev1: 1 });
        let (s2, v2) = fib_step(s1, 1);
        assert_eq!(v2, 2);
        assert_eq!(s2, FibState { prev2: 1, prev1: 2 });
        let (s3, v3) = fib_step(s2, 2);
        assert_eq!(v3, 3);
    }

    #[test]
    fn fib_seed_returns_0_for_n_0() {
        assert_eq!(fib_seed(0), 0);
    }

    #[test]
    fn fib_seed_returns_1_for_n_ge_1() {
        for n in [1u64, 2, 5, 100, u64::MAX] {
            assert_eq!(fib_seed(n), 1, "n={n} should produce seed value 1");
        }
    }

    #[test]
    fn fib_step_state_survives_simulated_restart() {
        // Drive 10 calls, capture the 10th state blob. Then
        // start a fresh call sequence using the captured state.
        let mut state = FibState { prev2: 0, prev1: 1 };
        let mut emitted: Vec<i128> = Vec::new();
        for i in 0..10u64 {
            let (new_state, v) = fib_step(state, i);
            emitted.push(v);
            state = new_state;
        }
        // 11th call should produce the 11th Fibonacci value.
        let (s10, v10) = fib_step(state, 10);
        // After 10 calls (1, 2, 3, 5, 8, 13, 21, 34, 55, 89), the
        // state is (55, 89). The 11th call emits 144.
        assert_eq!(v10, 144);
        assert_eq!(s10, FibState { prev2: 89, prev1: 144 });
    }
}
