use crate::Result;

pub trait EventBus {
    fn topic<F>(&self, callback: F) -> Result<()>
    where
        F: Fn(&Event) -> Result<()>;
    fn send(&self, event: &Event) -> Result<()>;
}

pub struct Event {}
