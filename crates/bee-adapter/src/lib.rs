//! `bee-adapter` — Bee Adapter contract (S16, ADR-0002).
//!
//! An Adapter is the plugin contract for talking to an external system.
//! Adapters are loaded into Bee (built-in for MVP, dynamically loaded
//! `cdylib` plugins later per ADR-0005). Two kinds exist:
//!
//! - [`InputAdapter`]: pulls events from an external source (subscribe).
//!   `next()` returns `Some(event)` while the stream is live and
//!   `None` to signal end-of-stream (per the S16 acceptance criterion).
//! - [`OutputAdapter`]: pushes events to an external sink (emit).
//!
//! For S16, the only built-in is [`FakeBinanceAdapter`]: a mock that
//! generates a configurable number of synthetic price events at 1Hz
//! (with the actual sleep optional to keep tests fast). The Datasource
//! / Adapter split (per ADR-0010) is layered on top in S17+.

use std::time::{SystemTime, UNIX_EPOCH};

/// A single event pulled from an Input Adapter or pushed to an Output
/// Adapter. Kept as a small struct so the bee-dsl-sql path can convert
/// to Arrow `RecordBatch` (S16: a one-row, two-column batch).
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub symbol: String,
    pub price: f64,
    pub ts_ms: i64,
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

pub type Result<T> = std::result::Result<T, AdapterError>;

/// Input (subscribe) adapter: pulls events from an external source.
///
/// `open` is async so an adapter can establish a network connection,
/// authenticate, or do other I/O before `next` is called. `next`
/// returns `Ok(None)` to signal end-of-stream.
pub trait InputAdapter: Send + 'static {
    type Config: Send + Sync;

    fn open(config: Self::Config) -> impl std::future::Future<Output = Result<Self>> + Send
    where
        Self: Sized;

    fn next(&mut self) -> impl std::future::Future<Output = Result<Option<Event>>> + Send;

    fn close(self) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Output (emit) adapter: pushes events to an external sink.
pub trait OutputAdapter: Send + 'static {
    type Config: Send + Sync;

    fn open(config: Self::Config) -> impl std::future::Future<Output = Result<Self>> + Send
    where
        Self: Sized;

    fn emit(&mut self, event: Event) -> impl std::future::Future<Output = Result<()>> + Send;

    fn close(self) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Configuration for [`FakeBinanceAdapter`].
#[derive(Debug, Clone)]
pub struct FakeBinanceConfig {
    pub symbol: String,
    pub max_events: u32,
    /// Optional per-event delay in milliseconds. `Some(0)` = no sleep
    /// (used by tests to keep them fast). Defaults to `Some(1000)`
    /// (1Hz) for production-like usage.
    pub delay_ms: Option<u64>,
    /// Optional deterministic seed (used by tests to assert specific
    /// values). `None` = use system time.
    pub base_ts_ms: Option<i64>,
}

impl Default for FakeBinanceConfig {
    fn default() -> Self {
        Self {
            symbol: "BTC/USDT".to_string(),
            max_events: 10,
            delay_ms: Some(1000),
            base_ts_ms: None,
        }
    }
}

/// Mock Input Adapter that emits `max_events` synthetic price events.
/// Each event's `price` increments by 1.0 starting at 100.0, so the
/// first event is `price=101.0` and the last is `price=100.0+N`.
///
/// `ts_ms` is `base_ts_ms` plus `count * 1000` if `base_ts_ms` is
/// given, else the system time when the event is generated. The base
/// ts makes tests deterministic.
pub struct FakeBinanceAdapter {
    symbol: String,
    max_events: u32,
    delay_ms: Option<u64>,
    base_ts_ms: Option<i64>,
    count: u32,
}

impl InputAdapter for FakeBinanceAdapter {
    type Config = FakeBinanceConfig;

    async fn open(config: Self::Config) -> Result<Self> {
        Ok(Self {
            symbol: config.symbol,
            max_events: config.max_events,
            delay_ms: config.delay_ms,
            base_ts_ms: config.base_ts_ms,
            count: 0,
        })
    }

    async fn next(&mut self) -> Result<Option<Event>> {
        if self.count >= self.max_events {
            return Ok(None);
        }
        if let Some(d) = self.delay_ms {
            if d > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(d)).await;
            }
        }
        self.count += 1;
        let price = 100.0 + self.count as f64;
        let ts_ms = match self.base_ts_ms {
            Some(base) => base + (self.count as i64 - 1) * 1000,
            None => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        };
        Ok(Some(Event {
            symbol: self.symbol.clone(),
            price,
            ts_ms,
        }))
    }

    async fn close(self) -> Result<()> {
        Ok(())
    }
}

/// Helper: run a [`FakeBinanceAdapter`] to completion and collect all
/// events. Equivalent to `open → loop(next) → close`, but inlined for
/// callers that don't need lifecycle hooks.
pub async fn collect_binance(
    config: FakeBinanceConfig,
) -> Result<Vec<Event>> {
    let mut adapter = FakeBinanceAdapter::open(config).await?;
    let mut events = Vec::with_capacity(adapter.max_events as usize);
    while let Some(ev) = adapter.next().await? {
        events.push(ev);
    }
    adapter.close().await?;
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_binance_adapter_emits_max_events_then_ends() {
        let cfg = FakeBinanceConfig {
            symbol: "BTC/USDT".to_string(),
            max_events: 10,
            delay_ms: Some(0),
            base_ts_ms: Some(1_000_000),
        };
        let events = collect_binance(cfg).await.unwrap();
        assert_eq!(events.len(), 10);
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.symbol, "BTC/USDT");
            assert_eq!(e.price, 101.0 + i as f64);
            assert_eq!(e.ts_ms, 1_000_000 + i as i64 * 1000);
        }
    }

    #[tokio::test]
    async fn fake_binance_adapter_with_zero_events_is_immediately_done() {
        let cfg = FakeBinanceConfig {
            symbol: "ETH/USDT".to_string(),
            max_events: 0,
            delay_ms: Some(0),
            base_ts_ms: Some(0),
        };
        let events = collect_binance(cfg).await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn fake_binance_adapter_close_is_noop() {
        let cfg = FakeBinanceConfig {
            max_events: 0,
            ..Default::default()
        };
        let adapter = FakeBinanceAdapter::open(cfg).await.unwrap();
        adapter.close().await.unwrap();
    }
}
