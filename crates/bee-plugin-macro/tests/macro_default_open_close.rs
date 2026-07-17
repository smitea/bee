//! S33.6.1 Task 2: lock down optional
//! `open` / `close` slots. An adapter with no
//! custom `open` (and/or no custom `close`)
//! should still compile + round-trip through
//! the vtable.

use bee_adapter::{AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;

#[derive(Default)]
pub struct MinimalAdapter {
    emitted: u32,
}

#[bee_adapter(input, name = "minimal")]
impl MinimalAdapter {
    // No `#[bee_method(slot = "open")]` — the
    // macro should generate a default open that
    // calls `Default::default()`.

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        if self.emitted >= 2 {
            return Ok(None);
        }
        self.emitted += 1;
        Ok(Some(Event {
            timestamp: 0,
            sequence: self.emitted as u64,
            payload: vec![self.emitted as u8],
        }))
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_open_compiles_and_works() {
    let vtable: *const _ = &MINIMAL_ADAPTER_VTABLE;
    let config: Vec<u8> = vec![];
    let ctx = unsafe {
        ((*vtable).open)(
            config.as_ptr(),
            config.len(),
            std::ptr::null_mut(),
        )
    };
    assert!(!ctx.is_null(), "default open must not return null");
    let mut out = EventBytes::EMPTY;
    let rc = unsafe { ((*vtable).next)(ctx, &mut out) };
    assert_eq!(rc, 1, "first next must return 1 event");
    let rc = unsafe { ((*vtable).close)(ctx) };
    assert_eq!(rc, 0);
}
