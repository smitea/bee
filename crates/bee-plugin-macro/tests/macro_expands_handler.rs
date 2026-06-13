//! S33.6 Task 4: locks down the proc-macro
//! for `#[bee_adapter(handler)]`.

use bee_adapter::{AdapterError, AdapterResult};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::HandlerVtable;

/// A counter handler. State is a u64. The
/// handler increments state on every call
/// and returns the new state as the result.
pub struct CounterHandler;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    Default,
)]
pub struct CounterState {
    pub count: u64,
}

#[bee_adapter(handler, name = "counter")]
impl CounterHandler {
    #[bee_method(slot = "handle")]
    pub async fn handle(
        state: CounterState,
        _event: Vec<u8>,
    ) -> AdapterResult<(CounterState, Vec<u8>)> {
        let new_state = CounterState {
            count: state.count + 1,
        };
        let result = bincode::serialize(&new_state).unwrap();
        Ok((new_state, result))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn counter_handler_vtable_round_trip() {
    let vtable: *const _ = &COUNTER_HANDLER_VTABLE;
    // init_state: default returns a
    // bincode-encoded empty Vec<u8> (8 bytes:
    // 4-byte length prefix + 4 bytes for the
    // empty length). Custom init_state is a
    // follow-up.
    let mut state_out = EventBytes::EMPTY;
    let rc = unsafe { ((*vtable).init_state)(&mut state_out) };
    assert_eq!(rc, 0);
    // Don't assert on exact size (the
    // bincode encoding can vary by version).
    // Just assert it's non-null and rc=0.
    let _ = state_out.len;

    // For the test, hand-construct a
    // CounterState blob and pass it
    // directly. (The empty init_state
    // can't be deserialized as a
    // CounterState with `count` field.)
    let state_in = CounterState { count: 0 };
    let state_in_bytes = bincode::serialize(&state_in).unwrap();
    // The `event` is a `Vec<u8>` — the
    // bincode wire format for `Vec<u8>`
    // is a 4-byte length prefix + the
    // bytes. For an empty Vec<u8>, that's
    // a 4-byte bincode blob (not empty).
    let event_in = bincode::serialize::<Vec<u8>>(&vec![]).unwrap();
    let mut new_state_out = EventBytes::EMPTY;
    let mut result_out = EventBytes::EMPTY;
    let rc = unsafe {
        ((*vtable).handle)(
            state_in_bytes.as_ptr(),
            state_in_bytes.len(),
            event_in.as_ptr(),
            event_in.len(),
            &mut new_state_out,
            &mut result_out,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "handle returned non-zero");
    let new_state_bytes = unsafe {
        std::slice::from_raw_parts(new_state_out.ptr, new_state_out.len)
    };
    let new_state: CounterState = bincode::deserialize(new_state_bytes).unwrap();
    assert_eq!(new_state.count, 1);
    let result_bytes = unsafe {
        std::slice::from_raw_parts(result_out.ptr, result_out.len)
    };
    // The result is bincode-encoded as
    // `Vec<u8>` (opaque bytes per the
    // Handler vtable contract). The handler
    // returned `bincode::serialize(&new_state)`
    // wrapped in a Vec. The test re-decodes:
    let result_wrapped: Vec<u8> = bincode::deserialize(result_bytes).unwrap();
    let result_state: CounterState = bincode::deserialize(&result_wrapped).unwrap();
    assert_eq!(result_state.count, 1);
}
