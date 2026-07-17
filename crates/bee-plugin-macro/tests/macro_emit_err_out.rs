//! S33.6.1 Task 3: lock down Output `emit`
//! with `err_out`. When the handler returns
//! `Err(AdapterError::Emit(msg))`, the FFI
//! writes an `Event { payload: msg.into_bytes() }`
//! to `err_out` and returns -1.

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;

pub struct FailingOutput;

#[bee_adapter(output, name = "fail-emit")]
impl FailingOutput {
    #[bee_method(slot = "open")]
    pub async fn open(_config: Vec<u8>) -> AdapterResult<Self> {
        Ok(Self)
    }

    #[bee_method(slot = "emit")]
    pub async fn emit_one(
        &mut self,
        event: Event,
    ) -> AdapterResult<()> {
        if event.payload.is_empty() {
            return Err(AdapterError::Emit("empty payload".into()));
        }
        Ok(())
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn emit_writes_err_to_err_out() {
    let vtable: *const _ = &FAILING_OUTPUT_VTABLE;
    let config: Vec<u8> = vec![];
    let ctx = unsafe {
        ((*vtable).open)(
            config.as_ptr(),
            config.len(),
            std::ptr::null_mut(),
        )
    };
    assert!(!ctx.is_null());

    // Send an event with empty payload — the handler
    // returns Err; the FFI writes to err_out and
    // returns -1.
    let event = Event {
        timestamp: 0,
        sequence: 1,
        payload: vec![],
    };
    let bytes = bincode::serialize(&event).unwrap();
    let mut err_out = EventBytes::EMPTY;
    let rc = unsafe {
        ((*vtable).emit)(
            ctx,
            bytes.as_ptr(),
            bytes.len(),
            &mut err_out,
        )
    };
    assert_eq!(rc, -1, "emit must return -1 on Err");
    assert!(err_out.len > 0, "err_out must be populated");
    let err_bytes = unsafe { std::slice::from_raw_parts(err_out.ptr, err_out.len) };
    let err_event: Event = bincode::deserialize(err_bytes)
        .expect("err_out must be a bincode-Event");
    let msg = String::from_utf8(err_event.payload).unwrap();
    assert!(msg.contains("empty payload"), "got err msg: {msg}");

    let rc = unsafe { ((*vtable).close)(ctx) };
    assert_eq!(rc, 0);
}
