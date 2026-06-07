//! Built-in Handlers for the S09 deploy path.
//!
//! These are reference Handlers the test pipeline uses; production code
//! would use Handler impls from a Plugin (loaded via bee-registry in S19+).
//!
//! - [`StartedHandler`] — logs once on first input, then forwards.
//! - [`TerminalHandler`] — records every received value into a shared
//!   `LogSink` so the test can assert on the final payload.

use std::future::Future;
use std::sync::{Arc, Mutex};

use bee_runtime::Handler;

use bee_runtime::RuntimeError;

#[derive(Clone, Default)]
pub struct LogSink {
    pub logs: Arc<Mutex<Vec<String>>>,
}

impl LogSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, msg: impl Into<String>) {
        self.logs.lock().unwrap().push(msg.into());
    }

    pub fn messages(&self) -> Vec<String> {
        self.logs.lock().unwrap().clone()
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.logs
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.contains(needle))
    }
}

#[derive(Clone)]
pub struct StartedHandler {
    pub tag: String,
    pub log: LogSink,
    pub started: bool,
}

impl StartedHandler {
    pub fn new(tag: impl Into<String>, log: LogSink) -> Self {
        Self {
            tag: tag.into(),
            log,
            started: false,
        }
    }
}

impl Handler for StartedHandler {
    type Input = i64;
    type Output = i64;

    fn handle(
        &mut self,
        input: i64,
    ) -> impl Future<Output = Result<Option<i64>, RuntimeError>> + Send {
        if !self.started {
            self.log
                .record(format!("{}: started (input={})", self.tag, input));
            self.started = true;
        }
        async move { Ok(Some(input)) }
    }

    fn finish(self) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}

#[derive(Clone)]
pub struct TerminalHandler {
    pub tag: String,
    pub log: LogSink,
    pub received: Arc<Mutex<Vec<i64>>>,
}

impl TerminalHandler {
    pub fn new(tag: impl Into<String>, log: LogSink) -> Self {
        Self {
            tag: tag.into(),
            log,
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn values(&self) -> Vec<i64> {
        self.received.lock().unwrap().clone()
    }
}

impl Handler for TerminalHandler {
    type Input = i64;
    type Output = i64;

    fn handle(
        &mut self,
        input: i64,
    ) -> impl Future<Output = Result<Option<i64>, RuntimeError>> + Send {
        self.log
            .record(format!("{}: received {}", self.tag, input));
        self.received.lock().unwrap().push(input);
        async move { Ok(None) }
    }

    fn finish(self) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}
