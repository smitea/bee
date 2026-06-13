//! S33.6 Task 2: locks down the proc-macro
//! for `#[bee_adapter(input)]`. The test
//! defines a sample adapter via the macro,
//! then exercises the generated vtable to
//! prove the FFI glue is correct.

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::InputAdapterVtable;

pub struct MockInput {
    count: u32,
    emitted: u32,
}

#[bee_adapter(input, name = "mock")]
impl MockInput {
    #[bee_method(slot = "open")]
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let c: u32 = bincode::deserialize(&config).unwrap_or(3);
        Ok(Self { count: c, emitted: 0 })
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        if self.emitted >= self.count { return Ok(None); }
        self.emitted += 1;
        Ok(Some(Event {
            timestamp: 0,
            sequence: self.emitted as u64,
            payload: self.emitted.to_string().into_bytes(),
        }))
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_input_vtable_round_trip() {
    let config = bincode::serialize(&3u32).unwrap();
    let vtable: *const _ = &MOCK_INPUT_VTABLE;
    let ctx = unsafe {
        ((*vtable).open)(
            config.as_ptr(),
            config.len(),
            std::ptr::null_mut(),
        )
    };
    assert!(!ctx.is_null(), "open returned null");

    for expected_seq in 1..=3u64 {
        let mut out = EventBytes::EMPTY;
        let rc = unsafe { ((*vtable).next)(ctx, &mut out) };
        assert_eq!(rc, 1, "expected 1 event on iteration {expected_seq}");
        assert!(!out.ptr.is_null());
        assert!(out.len > 0);
        let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
        let ev: Event = bincode::deserialize(bytes).expect("decode event");
        assert_eq!(ev.sequence, expected_seq);
    }
    let mut out = EventBytes::EMPTY;
    let rc = unsafe { ((*vtable).next)(ctx, &mut out) };
    assert_eq!(rc, 0, "expected end-of-stream on 4th call");

    let rc = unsafe { ((*vtable).close)(ctx) };
    assert_eq!(rc, 0);
}
