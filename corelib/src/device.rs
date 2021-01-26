use std::sync::Arc;

use crate::{Config, EventBus, Result};

pub trait Device {
    fn start(&self) -> Result<()>;

    fn stop(&self) -> Result<()>;
}

pub trait Driver {
    fn name() -> &'static str;
    fn version() -> &'static str;

    fn new_device<B, C>(config: &C, event_bus: &Arc<B>, target: &Target) -> Result<Box<dyn Device>>
    where
        B: EventBus,
        C: Config;
}

pub struct Target<'t> {
    pub id: &'t str,
}
