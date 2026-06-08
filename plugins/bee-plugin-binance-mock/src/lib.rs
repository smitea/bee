//! `bee-plugin-binance-mock` — S33 mock plugin.
//!
//! Implements the `binance_subscribe` Input Adapter. Generates
//! synthetic K-line events whose `price` follows a sine wave so
//! downstream MACD / EMA indicators are observable.
//!
//! ## Architecture
//!
//! - [`Factory`]: produces the [`bee_plugin_sdk::PluginManifest`]
//!   + [`bee_plugin_sdk::PluginHandle`] for the host.
//! - [`BinanceMockInput`]: the actual [`bee_adapter::InputAdapter`]
//!   implementation. Configurable cadence (default 1 event/sec)
//!   and sine-wave parameters.
//! - `cdylib_plugin!(Factory)` invocation at the bottom generates
//!   the FFI entry symbols.
//!
//! The mock is **synchronous** in the sense that each `next` call
//! returns one event (no background task). Real Binance WS would
//! push events; for the mock, the simulator controls timing.

use std::time::Duration;

use bee_adapter::{AdapterResult, Event, InputAdapter};
use bee_plugin_sdk::{
    AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName,
};

/// Configuration for the mock binance input.
#[derive(Debug, Clone)]
pub struct BinanceMockConfig {
    /// Symbol (e.g. "BTC/USDT"). Goes into the event payload
    /// prefix as ASCII bytes for inspection.
    pub symbol: String,
    /// Interval label (e.g. "5min"). Same — payload prefix.
    pub interval: String,
    /// Number of events to emit before signalling end-of-stream.
    pub count: u32,
    /// Per-event delay in milliseconds. `None` = no sleep (fast
    /// tests); `Some(ms)` = paced output.
    pub delay_ms: Option<u64>,
    /// Sine-wave amplitude in price units.
    pub amplitude: f64,
    /// Sine-wave base price (midline).
    pub base_price: f64,
    /// Sine-wave frequency (cycles per event).
    pub frequency: f64,
}

impl Default for BinanceMockConfig {
    fn default() -> Self {
        Self {
            symbol: "BTC/USDT".into(),
            interval: "5min".into(),
            count: 10,
            delay_ms: None,
            amplitude: 100.0,
            base_price: 30_000.0,
            frequency: 0.1,
        }
    }
}

/// Mock `binance_subscribe` Input Adapter. Emits `count` events
/// with a sine-wave `price` and synthetic K-line metadata.
pub struct BinanceMockInput {
    config: BinanceMockConfig,
    emitted: u32,
    started_at_ms: u64,
}

impl InputAdapter for BinanceMockInput {
    type Config = BinanceMockConfig;

    async fn open(config: Self::Config) -> AdapterResult<Self> {
        Ok(Self {
            config,
            emitted: 0,
            started_at_ms: Event::now_timestamp(),
        })
    }

    async fn next(&mut self) -> AdapterResult<Option<Event>> {
        if self.emitted >= self.config.count {
            return Ok(None);
        }
        if let Some(d) = self.config.delay_ms {
            if d > 0 {
                tokio::time::sleep(Duration::from_millis(d)).await;
            }
        }
        let sequence = self.emitted as u64;
        let t = sequence as f64;
        // Sine wave: base + amplitude * sin(2π * f * t).
        let price = self.config.base_price
            + self.config.amplitude
                * (2.0 * std::f64::consts::PI * self.config.frequency * t).sin();
        // Payload: ASCII "<symbol>,<interval>,<sequence>,<price>".
        // Keep the format stable so demo scripts can grep for it.
        let payload = format!(
            "{},{},{},{:.4}",
            self.config.symbol, self.config.interval, sequence, price
        )
        .into_bytes();
        self.emitted += 1;
        Ok(Some(Event {
            timestamp: self.started_at_ms + sequence * 1000,
            sequence,
            payload,
        }))
    }

    async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}

/// Factory for the binance mock plugin. Holds no state; both
/// methods are pure.
pub struct BinanceMockFactory;

impl Factory for BinanceMockFactory {
    fn manifest() -> PluginManifest {
        PluginManifest {
            name: PluginName("binance".into()),
            feature_version: "1.0.0".into(),
            abi_version: "v1".into(),
            adapters: vec![AdapterDescriptor {
                name: "subscribe".into(),
                is_input: true,
            }],
            handlers: vec![],
        }
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(BinanceMockFactory);

#[cfg(test)]
mod tests {
    use super::*;

    /// Local helper: open an `InputAdapter` and collect all
    /// events. Mirrors `bee_runtime::test_utils::collect_mock`
    /// but works on any `InputAdapter`.
    async fn collect_events(
        config: BinanceMockConfig,
    ) -> AdapterResult<Vec<Event>> {
        let mut adapter = BinanceMockInput::open(config).await?;
        let mut out = Vec::new();
        while let Some(e) = adapter.next().await? {
            out.push(e);
        }
        adapter.close().await?;
        Ok(out)
    }

    #[tokio::test]
    async fn emits_sine_wave_prices() {
        let config = BinanceMockConfig {
            count: 5,
            ..Default::default()
        };
        let events = collect_events(config).await.unwrap();
        assert_eq!(events.len(), 5);
        // First event is at t=0: price = base + amplitude * sin(0) = base.
        let first = String::from_utf8_lossy(&events[0].payload);
        assert!(
            first.starts_with("BTC/USDT,5min,0,"),
            "unexpected payload: {first}"
        );
        // Sequence is monotonic.
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.sequence, i as u64);
        }
    }

    #[tokio::test]
    async fn default_config_emits_ten_events() {
        let events = collect_events(BinanceMockConfig::default()).await.unwrap();
        assert_eq!(events.len(), 10);
    }

    #[tokio::test]
    async fn zero_count_means_empty_stream() {
        let config = BinanceMockConfig {
            count: 0,
            ..Default::default()
        };
        let events = collect_events(config).await.unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn factory_manifest_declares_subscribe_adapter() {
        let m = BinanceMockFactory::manifest();
        assert_eq!(m.name.0, "binance");
        assert_eq!(m.abi_version, "v1");
        assert_eq!(m.adapters.len(), 1);
        assert_eq!(m.adapters[0].name, "subscribe");
        assert!(m.adapters[0].is_input);
    }

    #[test]
    fn factory_init_returns_handle_with_manifest() {
        let h = BinanceMockFactory::init().unwrap();
        assert_eq!(h.manifest.name.0, "binance");
    }
}
