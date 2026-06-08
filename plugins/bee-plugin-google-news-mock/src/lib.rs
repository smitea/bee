//! `bee-plugin-google-news-mock` — S33 mock plugin.
//!
//! Implements the `google_news_search` Input Adapter. Generates
//! synthetic news article events whose payload is
//! `"<query>,<sequence>,<title>\n"`. The query is configurable
//! (default `"Bitcoin"`) and the title cycles through a small
//! fixed set of fake headlines.
//!
//! ## Architecture
//!
//! - [`Factory`]: produces the [`bee_plugin_sdk::PluginManifest`]
//!   + [`bee_plugin_sdk::PluginHandle`] for the host.
//! - [`GoogleNewsMockInput`]: the actual [`bee_adapter::InputAdapter`]
//!   implementation. Configurable query, count, and per-event delay.
//! - `cdylib_plugin!(Factory)` invocation at the bottom generates
//!   the FFI entry symbols.
//!
//! The mock is **synchronous** in the sense that each `next` call
//! returns one event (no background task). Real Google News RSS
//! would push events; for the mock, the simulator controls timing.

use std::time::Duration;

use bee_adapter::{AdapterResult, Event, InputAdapter};
use bee_plugin_sdk::{
    AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName,
};

/// Fixed set of fake news headlines the mock cycles through.
const FAKE_TITLES: &[&str] = &[
    "Bitcoin hits new high",
    "BTC adoption grows",
    "Crypto market update",
    "Bitcoin regulation news",
    "BTC price analysis",
];

/// Configuration for the mock google_news input.
#[derive(Debug, Clone)]
pub struct GoogleNewsMockConfig {
    /// Free-form query string the downstream pipeline filters on.
    /// Goes into the event payload prefix as ASCII bytes.
    pub query: String,
    /// Number of events to emit before signalling end-of-stream.
    pub count: u32,
    /// Per-event delay in milliseconds. `None` = no sleep (fast
    /// tests); `Some(ms)` = paced output.
    pub delay_ms: Option<u64>,
}

impl Default for GoogleNewsMockConfig {
    fn default() -> Self {
        Self {
            query: "Bitcoin".into(),
            count: 5,
            delay_ms: None,
        }
    }
}

/// Mock `google_news_search` Input Adapter. Emits `count` events
/// whose payload is `"<query>,<sequence>,<title>\n"` with the
/// title cycling through [`FAKE_TITLES`].
pub struct GoogleNewsMockInput {
    config: GoogleNewsMockConfig,
    emitted: u32,
    started_at_ms: u64,
}

impl InputAdapter for GoogleNewsMockInput {
    type Config = GoogleNewsMockConfig;

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
        let title = FAKE_TITLES[(sequence as usize) % FAKE_TITLES.len()];
        // Payload: ASCII "<query>,<sequence>,<title>\n".
        // Keep the format stable so demo scripts can grep for it.
        let payload = format!(
            "{},{},{}\n",
            self.config.query, sequence, title
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

/// Factory for the google_news mock plugin. Holds no state; both
/// methods are pure.
pub struct GoogleNewsMockFactory;

impl Factory for GoogleNewsMockFactory {
    fn manifest() -> PluginManifest {
        PluginManifest {
            name: PluginName("google_news".into()),
            feature_version: "1.0.0".into(),
            abi_version: "v1".into(),
            adapters: vec![AdapterDescriptor {
                name: "search".into(),
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

bee_plugin_sdk::cdylib_plugin!(GoogleNewsMockFactory);

#[cfg(test)]
mod tests {
    use super::*;

    /// Local helper: open an `InputAdapter` and collect all
    /// events. Mirrors `bee_runtime::test_utils::collect_mock`
    /// but works on any `InputAdapter`.
    async fn collect_events<C, A>(config: C) -> AdapterResult<Vec<Event>>
    where
        A: InputAdapter<Config = C>,
    {
        let mut adapter = A::open(config).await?;
        let mut out = Vec::new();
        while let Some(e) = adapter.next().await? {
            out.push(e);
        }
        adapter.close().await?;
        Ok(out)
    }

    #[tokio::test]
    async fn emits_synthetic_news_with_query() {
        let config = GoogleNewsMockConfig {
            query: "Ethereum".into(),
            count: 3,
            ..Default::default()
        };
        let events = collect_events::<_, GoogleNewsMockInput>(config)
            .await
            .unwrap();
        assert_eq!(events.len(), 3);
        // Sequence is monotonic.
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.sequence, i as u64);
        }
        // Every payload starts with "<query>,<sequence>," and
        // contains the query string.
        for (i, e) in events.iter().enumerate() {
            let payload = String::from_utf8_lossy(&e.payload);
            let expected_prefix = format!("Ethereum,{},", i);
            assert!(
                payload.starts_with(&expected_prefix),
                "unexpected payload: {payload}"
            );
            assert!(
                payload.contains("Ethereum"),
                "payload missing query: {payload}"
            );
        }
    }

    #[tokio::test]
    async fn default_config_emits_5_events() {
        let events = collect_events::<_, GoogleNewsMockInput>(
            GoogleNewsMockConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(events.len(), 5);
        // First event at sequence 0 starts with the default query.
        let first = String::from_utf8_lossy(&events[0].payload);
        assert!(
            first.starts_with("Bitcoin,0,"),
            "unexpected payload: {first}"
        );
    }

    #[tokio::test]
    async fn zero_count_is_empty() {
        let config = GoogleNewsMockConfig {
            count: 0,
            ..Default::default()
        };
        let events = collect_events::<_, GoogleNewsMockInput>(config)
            .await
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn factory_manifest_declares_search_adapter() {
        let m = GoogleNewsMockFactory::manifest();
        assert_eq!(m.name.0, "google_news");
        assert_eq!(m.abi_version, "v1");
        assert_eq!(m.adapters.len(), 1);
        assert_eq!(m.adapters[0].name, "search");
        assert!(m.adapters[0].is_input);
    }

    #[test]
    fn factory_init_returns_handle_with_manifest() {
        let h = GoogleNewsMockFactory::init().unwrap();
        assert_eq!(h.manifest.name.0, "google_news");
    }
}
