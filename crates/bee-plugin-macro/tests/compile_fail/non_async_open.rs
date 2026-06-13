//! S33.6 Task 6: trybuild compile-fail for
//! `#[bee_adapter(input)]` signature checks.
//! A non-async `open` should fail to compile.

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};

pub struct Bad;

#[bee_adapter(input, name = "bad")]
impl Bad {
    // BUG: not async.
    #[bee_method(slot = "open")]
    pub fn open(_config: Vec<u8>) -> AdapterResult<Self> {
        Ok(Self)
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        Ok(None)
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}

fn main() {}
