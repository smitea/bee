//! S33.6 Task 3: locks down the proc-macro
//! for `#[bee_adapter(output)]`.

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::OutputAdapterVtable;

pub struct MockOutput {
    received: u32,
}

#[bee_adapter(output, name = "mock-emit")]
impl MockOutput {
    #[bee_method(slot = "open")]
    pub async fn open(_config: Vec<u8>) -> AdapterResult<Self> {
        Ok(Self { received: 0 })
    }

    #[bee_method(slot = "emit")]
    pub async fn emit_one(&mut self, event: Event) -> AdapterResult<()> {
        assert_eq!(event.sequence, self.received as u64 + 1);
        self.received += 1;
        Ok(())
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_output_vtable_round_trip() {
    let config: Vec<u8> = vec![];
    let vtable: *const _ = &MOCK_OUTPUT_VTABLE;
    let ctx = unsafe {
        ((*vtable).open)(
            config.as_ptr(),
            config.len(),
            std::ptr::null_mut(),
        )
    };
    assert!(!ctx.is_null());

    for seq in 1..=3u64 {
        let event = Event {
            timestamp: 0,
            sequence: seq,
            payload: vec![],
        };
        let bytes = bincode::serialize(&event).unwrap();
        let rc = unsafe {
            ((*vtable).emit)(ctx, bytes.as_ptr(), bytes.len(), std::ptr::null_mut())
        };
        assert_eq!(rc, 0, "emit failed on seq {seq}");
    }
    let rc = unsafe { ((*vtable).close)(ctx) };
    assert_eq!(rc, 0);
}
