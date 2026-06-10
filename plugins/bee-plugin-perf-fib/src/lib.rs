//! `bee-plugin-perf-fib` — Fibonacci Handler UDFs for the S41
//! performance showcase.
//!
//! Two Handler UDFs are declared:
//! - [`fib_seed`]: stateless; returns 0 (n = 0) or 1 (n >= 1).
//! - [`fib_step`]: stateful; takes the previous two emitted values
//!   from a state blob and returns the next Fibonacci value,
//!   rolling the state forward.
//!
//! ## Architecture
//!
//! - [`FibState`]: the two-value rolling state (prev2, prev1) that
//!   the host keeps between calls (per the `HandlerVtable`
//!   contract — the host passes the previous blob in and the plugin
//!   returns the new blob).
//! - [`fib_seed`] / [`fib_step`]: pure compute functions exposed as
//!   Rust APIs (for in-process unit tests).
//! - `shim` module: `extern "C" fn handle` and `init_state` adapters
//!   that wrap the pure compute, following the same pattern as
//!   `bee-plugin-ta-indicators`.
//! - [`PerfFibFactory`]: produces the `PluginManifest` +
//!   `PluginHandle` for the host. The `init()` populates the
//!   `handlers` vtable map so the host can resolve `fib_seed` /
//!   `fib_step` and call through the FFI.
//! - `cdylib_plugin!(PerfFibFactory)` at the bottom generates the
//!   FFI entry symbols (`bee_plugin_init` / `bee_plugin_drop`).

use std::sync::Arc;

use bee_plugin_sdk::{
    event::EventBytes, Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName,
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

/// `fib_seed(n)`: returns 0 if `n == 0`, else 1.
///
/// This is the seed for the Fibonacci sequence: it produces the
/// first emitted value for a Task. The runtime is expected to
/// invoke `fib_seed(0)` exactly once per Task, then call
/// `fib_step(n)` for n = 1, 2, 3, … to continue the sequence.
pub fn fib_seed(n: u64) -> i128 {
    if n == 0 { 0 } else { 1 }
}

/// `fib_step(state, n) -> (new_state, value)`: stateful Fibonacci
/// step. `state` is the previous call's state blob (or empty for
/// the first call). The function rolls the state forward and
/// returns the new state alongside the new Fibonacci value.
///
/// A corrupted state is treated as the seed pair `(0, 1)` — the
/// same lenient semantics as the original KV-backed impl: a Task
/// that restarts mid-sequence resumes from the seed rather than
/// panicking.
pub fn fib_step(state: FibState, _n: u64) -> (FibState, i128) {
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

// ---------------------------------------------------------------------------
// FFI shims — wrap the pure compute for the HandlerVtable
// ---------------------------------------------------------------------------
//
// Pattern mirrors `bee-plugin-ta-indicators`:
// - `init_state` writes a fresh `FibState` (the seed pair `(0, 1)`)
//   to `*out` as bincode bytes.
// - `handle` decodes the input state (or defaults to seed on
//   empty / corrupt), decodes the event `n: u64`, calls `fib_step`,
//   writes the new state and the new Fibonacci value to the out
//   pointers, returns 0.
// - All bytes that the host will read are produced by
//   `bincode::serialize` into a `Vec<u8>`; we leak the `Vec` to
//   satisfy the vtable contract ("the producer owns the bytes
//   for the consumer's read; the host frees via its allocator
//   after reading"). For the perf-fib demo the values stay
//   small (one `FibState` + one `i128`), so the leak is
//   negligible over the program's lifetime.

/// Serialize `value` to bincode, leak the `Vec<u8>`, and write
/// `(ptr, len)` into `*out`. Returns 0 on success, -1 on
/// serialization failure.
fn write_event_bytes<T: Serialize>(out: *mut EventBytes, value: &T) -> i32 {
    let bytes = match bincode::serialize(value) {
        Ok(b) => b,
        Err(_) => return -1,
    };
    let len = bytes.len();
    let ptr = bytes.as_ptr();
    std::mem::forget(bytes);
    unsafe { *out = EventBytes { ptr, len } };
    0
}

/// Deserialize a bincode-encoded value from `(ptr, len)`. On
/// empty input or any decode failure, returns `Default::default()`
/// — the lenient semantics described in [`fib_step`].
fn decode_or_default<T: Default + for<'de> Deserialize<'de>>(ptr: *const u8, len: usize) -> T {
    if len == 0 {
        return T::default();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    bincode::deserialize(bytes).unwrap_or_default()
}

/// `fib_seed` Handler Vtable: stateless. `state` is ignored;
/// `event` is the bincode-encoded `n: u64`; result is the
/// bincode-encoded `i128` Fibonacci value. The new state is the
/// same seed pair (stateless → no state change).
pub mod seed_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        // Stateless, but the vtable contract requires a state
        // blob. Use the empty `FibState` as a sentinel (the
        // stateless seed handler ignores it).
        write_event_bytes(out, &FibState { prev2: 0, prev1: 0 })
    }

    pub unsafe extern "C" fn handle(
        _state_ptr: *const u8,
        _state_len: usize,
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        let n: u64 = decode_or_default(event_ptr, event_len);
        let value = fib_seed(n);
        // Stateless handler: re-write the same sentinel state.
        if write_event_bytes(new_state_out, &FibState { prev2: 0, prev1: 0 }) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &value)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

/// `fib_step` Handler Vtable: stateful. `state` is the previous
/// `FibState`; `event` is the bincode-encoded `n: u64`; the
/// result is the bincode-encoded `i128` Fibonacci value. The
/// new state is the rolled-forward `FibState` so the next
/// call sees the correct window.
pub mod step_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        // Seed the window as (0, 1) so the first call's
        // `prev2 + prev1 = 0 + 1 = 1` (the canonical Fibonacci
        // first value). `fib_step` re-checks the seed
        // sentinel at runtime so this initialization is
        // cosmetic.
        write_event_bytes(out, &FibState { prev2: 0, prev1: 1 })
    }

    pub unsafe extern "C" fn handle(
        state_ptr: *const u8,
        state_len: usize,
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        let state: FibState = decode_or_default(state_ptr, state_len);
        let n: u64 = decode_or_default(event_ptr, event_len);
        let (new_state, value) = fib_step(state, n);
        if write_event_bytes(new_state_out, &new_state) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &value)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
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
/// state lives in the host's per-UDF `Mutex<Vec<u8>>` (per the
/// `HandlerVtable` contract).
pub struct PerfFibFactory;

impl Factory for PerfFibFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        let mut handlers: std::collections::HashMap<
            String,
            *const bee_plugin_sdk::vtable::HandlerVtable,
        > = std::collections::HashMap::new();
        handlers.insert("fib_seed".to_string(), &seed_shim::VTABLE as *const _);
        handlers.insert("fib_step".to_string(), &step_shim::VTABLE as *const _);
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: Arc::new(()),
            input_adapters: std::collections::HashMap::new(),
            output_adapters: std::collections::HashMap::new(),
            handlers,
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(PerfFibFactory);

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::MaybeUninit;

    type ShimHandleFn = unsafe extern "C" fn(
        *const u8,
        usize,
        *const u8,
        usize,
        *mut EventBytes,
        *mut EventBytes,
        *mut EventBytes,
    ) -> i32;

    fn call_handle(
        handle_fn: ShimHandleFn,
        state_bytes: &[u8],
        event_bytes: &[u8],
    ) -> (Vec<u8>, Vec<u8>) {
        let mut new_state_eb = MaybeUninit::<EventBytes>::zeroed();
        let mut result_eb = MaybeUninit::<EventBytes>::zeroed();
        let mut err_eb = MaybeUninit::<EventBytes>::zeroed();
        let rc = unsafe {
            handle_fn(
                state_bytes.as_ptr(),
                state_bytes.len(),
                event_bytes.as_ptr(),
                event_bytes.len(),
                new_state_eb.as_mut_ptr(),
                result_eb.as_mut_ptr(),
                err_eb.as_mut_ptr(),
            )
        };
        assert_eq!(rc, 0, "handle shim returned {rc}");
        let new_state_eb = unsafe { new_state_eb.assume_init() };
        let result_eb = unsafe { result_eb.assume_init() };
        assert!(!new_state_eb.ptr.is_null());
        assert!(!result_eb.ptr.is_null());
        let new_state = unsafe {
            std::slice::from_raw_parts(new_state_eb.ptr, new_state_eb.len).to_vec()
        };
        let result = unsafe {
            std::slice::from_raw_parts(result_eb.ptr, result_eb.len).to_vec()
        };
        (new_state, result)
    }

    fn call_init(init_fn: unsafe extern "C" fn(*mut EventBytes) -> i32) -> Vec<u8> {
        let mut state_eb = MaybeUninit::<EventBytes>::zeroed();
        let rc = unsafe { init_fn(state_eb.as_mut_ptr()) };
        assert_eq!(rc, 0);
        let state_eb = unsafe { state_eb.assume_init() };
        assert!(!state_eb.ptr.is_null());
        unsafe { std::slice::from_raw_parts(state_eb.ptr, state_eb.len).to_vec() }
    }

    fn encode<T: Serialize>(v: &T) -> Vec<u8> {
        bincode::serialize(v).expect("bincode")
    }

    #[test]
    fn step_shim_first_call_returns_1() {
        // Initial state: (0, 1). next = 0+1 = 1.
        let init = call_init(step_shim::init_state);
        let ev = encode(&0u64);
        let (new_state, result_bytes) = call_handle(step_shim::handle, &init, &ev);
        let v: i128 = bincode::deserialize(&result_bytes).unwrap();
        assert_eq!(v, 1);
        let s: FibState = bincode::deserialize(&new_state).unwrap();
        assert_eq!(s, FibState { prev2: 1, prev1: 1 });
    }

    #[test]
    fn step_shim_computes_first_known_fibonacci_values() {
        // Drive the shim through 20 calls with n=0,1,...,19.
        // Initial state (0, 1) → emit 1. New state (1, 1).
        // Call 2: (1, 1) → emit 2. New state (1, 2).
        // Call 3: (1, 2) → emit 3. New state (2, 3).
        // ... up to call 20 → emit 6765 (the 20th Fibonacci number).
        let mut state = call_init(step_shim::init_state);
        let mut values: Vec<i128> = Vec::with_capacity(20);
        for i in 0..20u64 {
            let ev = encode(&i);
            let (new_state, result_bytes) = call_handle(step_shim::handle, &state, &ev);
            let v: i128 = bincode::deserialize(&result_bytes).unwrap();
            values.push(v);
            state = new_state;
        }
        let expected: Vec<i128> = vec![
            1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765,
            10946,
        ];
        assert_eq!(values, expected);
    }

    #[test]
    fn step_shim_state_survives_simulated_restart() {
        // Drive 10 calls, capture the 10th state blob. Then
        // start a fresh shim call sequence using the captured
        // state blob as the init. The 11th call should produce
        // the same value as the original continuous sequence's
        // 11th value.
        let mut state = call_init(step_shim::init_state);
        let mut emitted: Vec<i128> = Vec::new();
        for i in 0..10u64 {
            let ev = encode(&i);
            let (new_state, result_bytes) = call_handle(step_shim::handle, &state, &ev);
            emitted.push(bincode::deserialize(&result_bytes).unwrap());
            state = new_state;
        }
        // Restart with the captured state.
        let resumed_state = state;
        let ev10 = encode(&10u64);
        let (_, result_bytes) = call_handle(step_shim::handle, &resumed_state, &ev10);
        let v10: i128 = bincode::deserialize(&result_bytes).unwrap();
        // 11th Fibonacci value: 144 (1, 2, 3, 5, 8, 13, 21, 34, 55,
        // 89, 144 — call #10 emitted 89, call #11 emits 144).
        let expected_10: i128 = 144;
        assert_eq!(v10, expected_10);
    }

    #[test]
    fn step_shim_corrupt_state_resets_to_seed() {
        // The lenient semantics: a corrupt state blob is treated
        // as the seed pair (0, 1), so the next call emits 1.
        let corrupt = vec![0xFF, 0xFF, 0xFF, 0xFF];
        let ev = encode(&0u64);
        let (_, result_bytes) = call_handle(step_shim::handle, &corrupt, &ev);
        let v: i128 = bincode::deserialize(&result_bytes).unwrap();
        assert_eq!(v, 1, "corrupt state must reset to seed and emit 1");
    }

    #[test]
    fn seed_shim_returns_0_for_n_0() {
        let init = call_init(seed_shim::init_state);
        let ev = encode(&0u64);
        let (_, result_bytes) = call_handle(seed_shim::handle, &init, &ev);
        let v: i128 = bincode::deserialize(&result_bytes).unwrap();
        assert_eq!(v, 0);
    }

    #[test]
    fn seed_shim_returns_1_for_n_ge_1() {
        let init = call_init(seed_shim::init_state);
        for n in [1u64, 2, 5, 100, u64::MAX] {
            let ev = encode(&n);
            let (_, result_bytes) = call_handle(seed_shim::handle, &init, &ev);
            let v: i128 = bincode::deserialize(&result_bytes).unwrap();
            assert_eq!(v, 1, "n={n} should produce seed value 1");
        }
    }

    #[test]
    fn fib_step_pure_function_matches_known_values() {
        // Direct test of the pure `fib_step` (no FFI).
        let s0 = FibState { prev2: 0, prev1: 0 }; // sentinel
        let (s1, v1) = fib_step(s0, 0);
        assert_eq!(v1, 1);
        assert_eq!(s1, FibState { prev2: 1, prev1: 1 });
        let (s2, v2) = fib_step(s1, 1);
        assert_eq!(v2, 2);
        assert_eq!(s2, FibState { prev2: 1, prev1: 2 });
        let (s3, v3) = fib_step(s2, 2);
        assert_eq!(v3, 3);
    }
}
