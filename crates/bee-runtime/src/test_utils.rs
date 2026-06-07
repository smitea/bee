//! `bee-runtime::test_utils` — generic test fixtures for Bee.
//!
//! Per the S16 spec, Bee's `bee-runtime` is **business-agnostic**: it
//! defines the *mechanism* (Handler, Phase, Dag, Runtime) and a
//! generic `MockInputAdapter` for testing that mechanism. Concrete
//! business Datasources (Binance, CoinGecko, InfluxDB, etc.) ship as
//! **external plugins** in their own crates and are **not** compiled
//! into the Bee binary — see S19 (Plugin trait + libloading).
//!
//! Anything in this module is a test fixture: it is `pub` so other
//! crates' integration tests can use it, but it is not part of the
//! production runtime surface.

pub use bee_adapter::{AdapterError, AdapterResult, Event, InputAdapter, OutputAdapter};

use std::time::Duration;

/// Configuration for [`MockInputAdapter`]. MVP: how many events to
/// emit, an optional per-event delay, and a payload template.
#[derive(Debug, Clone)]
pub struct MockInputConfig {
    /// Total number of events to emit before `next` returns `Ok(None)`.
    pub count: u32,
    /// Payload bytes for each event. If `None`, the Adapter fills
    /// the payload with the sequence number as ASCII bytes (so tests
    /// can assert content by index without bringing in a serializer).
    pub payload: Option<Vec<u8>>,
    /// Optional per-event delay in milliseconds. `None` or `Some(0)`
    /// = no sleep (default for tests, keeps them fast).
    pub delay_ms: Option<u64>,
    /// Optional deterministic starting timestamp (ms since epoch).
    /// `None` = use `Event::now_timestamp()` for each event.
    pub base_timestamp_ms: Option<u64>,
}

impl Default for MockInputConfig {
    fn default() -> Self {
        Self {
            count: 10,
            payload: None,
            delay_ms: None,
            base_timestamp_ms: None,
        }
    }
}

/// Generic test-fixture `InputAdapter` — produces `count` synthetic
/// events then signals end-of-stream. The events use the generic
/// [`Event`] envelope (timestamp, sequence, payload); no domain
/// semantics.
///
/// Real business adapters (Binance etc.) implement the same
/// [`InputAdapter`] trait but live in their own plugin crates; this
/// Adapter is the **only** InputAdapter compiled into the Bee binary.
pub struct MockInputAdapter {
    count: u32,
    payload: Option<Vec<u8>>,
    delay_ms: Option<u64>,
    base_timestamp_ms: Option<u64>,
    emitted: u32,
}

impl InputAdapter for MockInputAdapter {
    type Config = MockInputConfig;

    async fn open(config: Self::Config) -> AdapterResult<Self> {
        Ok(Self {
            count: config.count,
            payload: config.payload,
            delay_ms: config.delay_ms,
            base_timestamp_ms: config.base_timestamp_ms,
            emitted: 0,
        })
    }

    async fn next(&mut self) -> AdapterResult<Option<Event>> {
        if self.emitted >= self.count {
            return Ok(None);
        }
        if let Some(d) = self.delay_ms {
            if d > 0 {
                tokio::time::sleep(Duration::from_millis(d)).await;
            }
        }
        let sequence = self.emitted as u64;
        self.emitted += 1;
        let timestamp = self
            .base_timestamp_ms
            .map(|b| b + sequence * 1000)
            .unwrap_or_else(Event::now_timestamp);
        let payload = match &self.payload {
            Some(p) => p.clone(),
            None => sequence.to_string().into_bytes(),
        };
        Ok(Some(Event {
            timestamp,
            sequence,
            payload,
        }))
    }

    async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}

/// Helper: run a [`MockInputAdapter`] to completion and collect all
/// events. Equivalent to `open → loop(next) → close`.
pub async fn collect_mock(
    config: MockInputConfig,
) -> AdapterResult<Vec<Event>> {
    let mut adapter = MockInputAdapter::open(config).await?;
    let mut events = Vec::with_capacity(adapter.count as usize);
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
    async fn mock_input_adapter_emits_n_events_then_ends() {
        let cfg = MockInputConfig {
            count: 5,
            base_timestamp_ms: Some(1_000_000),
            ..Default::default()
        };
        let events = collect_mock(cfg).await.unwrap();
        assert_eq!(events.len(), 5);
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.sequence, i as u64);
            assert_eq!(e.timestamp, 1_000_000 + i as u64 * 1000);
            assert_eq!(e.payload, i.to_string().into_bytes());
        }
    }

    #[tokio::test]
    async fn mock_input_adapter_with_zero_events_is_immediately_done() {
        let events = collect_mock(MockInputConfig {
            count: 0,
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn mock_input_adapter_close_is_noop() {
        let adapter = MockInputAdapter::open(MockInputConfig {
            count: 0,
            ..Default::default()
        })
        .await
        .unwrap();
        adapter.close().await.unwrap();
    }

    #[tokio::test]
    async fn mock_input_adapter_custom_payload_repeats() {
        let events = collect_mock(MockInputConfig {
            count: 3,
            payload: Some(b"P".to_vec()),
            base_timestamp_ms: Some(0),
            ..Default::default()
        })
        .await
        .unwrap();
        for e in &events {
            assert_eq!(e.payload, b"P");
        }
    }
}
