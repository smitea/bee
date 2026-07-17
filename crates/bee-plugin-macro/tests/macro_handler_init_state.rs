//! S33.6.1 Task 1: lock down Handler
//! `#[bee_method(slot = "init_state")]` support.
//! The macro should generate an `init_state`
//! FFI fn that returns the bincode-encoded
//! custom state (not just empty Vec<u8>).

use bee_adapter::AdapterResult;
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::HandlerVtable;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterState {
    pub count: u64,
    pub label: String,
}

pub struct CounterHandler;

#[bee_adapter(handler, name = "counter")]
impl CounterHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<CounterState> {
        Ok(CounterState {
            count: 0,
            label: "starting".into(),
        })
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        mut state: CounterState,
        _event: Vec<u8>,
    ) -> AdapterResult<(CounterState, Vec<u8>)> {
        state.count += 1;
        let result = bincode::serialize(&state.count).unwrap();
        Ok((state, result))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handler_init_state_returns_custom_state() {
    let vtable: *const _ = &COUNTER_HANDLER_VTABLE;
    let mut out = EventBytes::EMPTY;
    let rc = unsafe { ((*vtable).init_state)(&mut out) };
    assert_eq!(rc, 0, "init_state must return 0");
    assert!(out.len > 0, "init_state must return non-empty bytes");
    let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
    let state: CounterState = bincode::deserialize(bytes)
        .expect("must bincode-decode as CounterState");
    assert_eq!(state.count, 0);
    assert_eq!(state.label, "starting");
}
