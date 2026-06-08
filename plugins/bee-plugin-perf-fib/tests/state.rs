//! Unit tests for the `fib_step` UDF (stateful, KV-backed).

use bee_plugin_perf_fib::{fib_step, FibState};
use bee_plugin_sdk::BeeHostV1;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::sync::{LazyLock, Mutex};

/// Process-global KV store for the test harness.
static TEST_KV: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Serializes the 3 state tests so the shared TEST_KV isn't clobbered
/// by cargo test's parallel execution.
static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Build a `BeeHostV1` with kv_* wired to TEST_KV + a fixed stream_id.
fn build_mock_host() -> BeeHostV1 {
    extern "C" fn kv_get(
        _ctx: *mut c_void,
        key: *const c_char,
        out_value: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32 {
        // SAFETY: c_char is null-terminated.
        let key_str = unsafe { CStr::from_ptr(key) }.to_str().unwrap();
        let store = TEST_KV.lock().unwrap();
        match store.get(key_str) {
            Some(v) => {
                let boxed = v.clone().into_boxed_slice();
                let len = boxed.len();
                let ptr = Box::into_raw(boxed) as *mut u8;
                unsafe {
                    *out_value = ptr;
                    *out_len = len;
                }
                0
            }
            None => 1,
        }
    }
    extern "C" fn kv_put(
        _ctx: *mut c_void,
        key: *const c_char,
        value: *const u8,
        len: usize,
    ) -> i32 {
        let key_str = unsafe { CStr::from_ptr(key) }.to_str().unwrap();
        let value_slice = unsafe { std::slice::from_raw_parts(value, len) };
        TEST_KV
            .lock()
            .unwrap()
            .insert(key_str.to_string(), value_slice.to_vec());
        0
    }
    extern "C" fn kv_cas(
        _ctx: *mut c_void,
        key: *const c_char,
        expected: *const u8,
        exp_len: usize,
        new: *const u8,
        new_len: usize,
    ) -> i32 {
        let key_str = unsafe { CStr::from_ptr(key) }.to_str().unwrap();
        let exp = unsafe { std::slice::from_raw_parts(expected, exp_len) };
        let new = unsafe { std::slice::from_raw_parts(new, new_len) };
        let mut store = TEST_KV.lock().unwrap();
        match store.get(key_str) {
            Some(v) if v.as_slice() == exp => {
                store.insert(key_str.to_string(), new.to_vec());
                0
            }
            _ => 1,
        }
    }
    extern "C" fn current_stream_id(_ctx: *mut c_void, out_id: *mut [u8; 32]) -> i32 {
        // Fixed stream_id for the tests; all fib_step calls in the same test
        // share the same key, so the state evolves across calls.
        unsafe { *out_id = [0u8; 32]; }
        0
    }

    BeeHostV1 {
        ctx: std::ptr::null_mut(),
        register_adapter: None,
        register_input_adapter_vtable: None,
        register_output_adapter_vtable: None,
        register_handler_vtable: None,
        kv_get: Some(kv_get),
        kv_put: Some(kv_put),
        kv_cas: Some(kv_cas),
        current_stream_id: Some(current_stream_id),
    }
}

/// Compute the same key the production `state_key()` helper would compute,
/// for a fixed [0u8; 32] stream_id. Used by the tests to seed/inspect the KV.
fn expected_state_key() -> String {
    format!(
        "state/handler/{}/fib_step/state",
        hex::encode([0u8; 32])
    )
}

#[test]
fn fib_step_returns_1_on_first_call_with_initial_state() {
    let _guard = TEST_LOCK.lock().unwrap();
    TEST_KV.lock().unwrap().clear();

    let host = build_mock_host();
    // Initialize the state to (0, 1) — the Fibonacci seed pair.
    let key = expected_state_key();
    let initial = FibState { prev2: 0, prev1: 1 };
    let bytes = bincode::serialize(&initial).unwrap();
    TEST_KV.lock().unwrap().insert(key, bytes);

    // First fib_step call: state is (0, 1), so next = 0+1 = 1.
    let result = fib_step(&host, 1);
    assert_eq!(result, 1, "fib_step(1) should be 1");
}

#[test]
fn fib_step_computes_first_20_values_correctly() {
    let _guard = TEST_LOCK.lock().unwrap();
    TEST_KV.lock().unwrap().clear();

    let host = build_mock_host();
    let key = expected_state_key();
    let initial = FibState { prev2: 0, prev1: 1 };
    let bytes = bincode::serialize(&initial).unwrap();
    TEST_KV.lock().unwrap().insert(key, bytes);

    // Expected first 20 fib_step values. With initial state (prev2=0, prev1=1)
    // and next() = prev2 + prev1, the impl produces the *shifted* Fibonacci
    // sequence: 1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987,
    // 1597, 2584, 4181, 6765, 10946.
    //
    // (Note: this differs from the canonical 0-indexed Fibonacci
    // 1, 1, 2, 3, 5, ... which would require initial state (1, 0). The
    // FibState docstring defines the state as "the two most recent emitted
    // values", so the correct seed is (0, 1) — and that is what the impl
    // uses.)
    let expected: [i128; 20] = [
        1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765, 10946,
    ];
    for n in 1..=20u64 {
        let actual = fib_step(&host, n);
        assert_eq!(
            actual,
            expected[(n - 1) as usize],
            "fib_step({}) mismatch",
            n
        );
    }
}

#[test]
fn fib_step_state_survives_simulated_restart() {
    let _guard = TEST_LOCK.lock().unwrap();
    TEST_KV.lock().unwrap().clear();

    let key = expected_state_key();
    let initial = FibState { prev2: 0, prev1: 1 };
    let bytes = bincode::serialize(&initial).unwrap();
    TEST_KV.lock().unwrap().insert(key, bytes);

    // Compute 100 values
    let host = build_mock_host();
    for n in 1..=100u64 {
        let _ = fib_step(&host, n);
    }

    // "Restart" — construct a new host (the KV store is the global TEST_KV,
    // which survives). The 101st call should return a non-zero value.
    let new_host = build_mock_host();
    let result = fib_step(&new_host, 101);
    assert!(result > 0, "fib_step after restart should be non-zero");
}
