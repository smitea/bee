//! `bee-adapter` — Bee Adapter contract (S16, ADR-0002 / ADR-0003 / ADR-0010).
//!
//! An Adapter is the plugin contract for talking to an external system.
//! Adapters are loaded into Bee via the Plugin SDK (cdylib + libloading,
//! ADR-0005/0009). Two kinds exist:
//!
//! - [`InputAdapter`]: pulls events from an external source (subscribe).
//!   `next()` returns `Some(event)` while the stream is live and
//!   `None` to signal end-of-stream.
//! - [`OutputAdapter`]: pushes events to an external sink (emit).
//!
//! ## Bee core is business-agnostic
//!
//! This crate defines the **mechanism** (the trait + `Event` envelope)
//! only. It contains **no** domain-specific implementations —
//! Binance / CoinGecko / InfluxDB etc. ship as **external plugins** in
//! their own crates, NOT compiled into the Bee binary. The test
//! fixture that exercises the trait is the generic
//! `MockInputAdapter` in `bee_runtime::test_utils` (per S16 spec).
//!
//! ## Event envelope
//!
//! The wire format is intentionally generic: a timestamp, a monotonic
//! sequence number (per-Adapter), and an opaque `payload: Vec<u8>`.
//! Domain semantics (price, symbol, sentiment score, etc.) live in
//! the payload encoding chosen by each plugin author.

use std::time::{SystemTime, UNIX_EPOCH};

/// A single event pulled from an Input Adapter or pushed to an Output
/// Adapter. The `payload` is opaque bytes — the Adapter author picks
/// the encoding (JSON, protobuf, Arrow, raw struct bytes, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Wall-clock timestamp in milliseconds since the unix epoch.
    /// Producers should set this to the time the event was observed
    /// in the upstream system (not the time it was pulled into Bee).
    pub timestamp: u64,
    /// Monotonic sequence number, scoped to a single Adapter
    /// instance. Useful for ordering, deduplication, and
    /// checkpoint offsets. Starts at 0.
    pub sequence: u64,
    /// Opaque payload bytes. Domain semantics are defined by the
    /// Adapter author.
    pub payload: Vec<u8>,
}

impl Event {
    /// Convenience: current system time as `timestamp` (ms since epoch).
    pub fn now_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("open: {0}")]
    Open(String),
    #[error("next: {0}")]
    Next(String),
    #[error("emit: {0}")]
    Emit(String),
    #[error("close: {0}")]
    Close(String),
}

pub type AdapterResult<T> = std::result::Result<T, AdapterError>;

/// Input (subscribe) adapter: pulls events from an external source.
///
/// `open` is async so an adapter can establish a network connection,
/// authenticate, or do other I/O before `next` is called. `next`
/// returns `Ok(None)` to signal end-of-stream.
pub trait InputAdapter: Send + 'static {
    type Config: Send + Sync;

    fn open(config: Self::Config) -> impl std::future::Future<Output = AdapterResult<Self>> + Send
    where
        Self: Sized;

    fn next(&mut self) -> impl std::future::Future<Output = AdapterResult<Option<Event>>> + Send;

    fn close(self) -> impl std::future::Future<Output = AdapterResult<()>> + Send;
}

/// Output (emit) adapter: pushes events to an external sink.
pub trait OutputAdapter: Send + 'static {
    type Config: Send + Sync;

    fn open(config: Self::Config) -> impl std::future::Future<Output = AdapterResult<Self>> + Send
    where
        Self: Sized;

    fn emit(&mut self, event: Event) -> impl std::future::Future<Output = AdapterResult<()>> + Send;

    fn close(self) -> impl std::future::Future<Output = AdapterResult<()>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_constructs_and_compares() {
        let a = Event {
            timestamp: 1_000_000,
            sequence: 7,
            payload: b"hi".to_vec(),
        };
        let b = a.clone();
        assert_eq!(a, b);
        assert_eq!(a.timestamp, 1_000_000);
        assert_eq!(a.sequence, 7);
        assert_eq!(a.payload, b"hi");
    }

    #[test]
    fn event_now_timestamp_is_recent() {
        let t = Event::now_timestamp();
        // Any plausible 2024+ timestamp in ms; we don't assert a
        // tight window to avoid CI flake.
        assert!(t > 1_700_000_000_000, "now_timestamp returned {t}");
    }
}
