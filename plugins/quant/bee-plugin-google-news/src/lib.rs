//! `bee-plugin-google-news` — production-grade reference implementation.
//!
//! Implements the `google_news_search` Input Adapter. Generates
//! synthetic news article events whose payload is
//! `"<query>,<sequence>,<title>\n"`. The query is configurable
//! (default `"Bitcoin"`) and the title cycles through a small
//! fixed set of fake headlines. Plugin scaffold is
//! production-grade (cdylib + FFI vtable); the actual data source
//! is a synthetic-news placeholder that will be replaced by the
//! real NewsAPI HTTP client in S35.
//!
//! ## Architecture
//!
//! - [`Factory`]: produces the [`bee_plugin_sdk::PluginManifest`]
//!   + [`bee_plugin_sdk::PluginHandle`] for the host.
//! - [`GoogleNewsInput`]: the actual [`bee_adapter::InputAdapter`]
//!   implementation. Configurable query, count, and per-event delay.
//! - `cdylib_plugin!(Factory)` invocation at the bottom generates
//!   the FFI entry symbols.
//!
//! The placeholder is **synchronous** in the sense that each `next`
//! call returns one event (no background task). Real Google News
//! RSS would push events; for the placeholder, the simulator
//! controls timing.

use std::time::Duration;

use bee_adapter::{AdapterResult, Event, InputAdapter};
use bee_plugin_sdk::{
    vtable::InputAdapterVtable, AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName,
};

/// Fixed set of fake news headlines the placeholder cycles through.
const FAKE_TITLES: &[&str] = &[
    "Bitcoin hits new high",
    "BTC adoption grows",
    "Crypto market update",
    "Bitcoin regulation news",
    "BTC price analysis",
];

/// Configuration for the google_news input.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GoogleNewsConfig {
    /// Free-form query string the downstream pipeline filters on.
    /// Goes into the event payload prefix as ASCII bytes.
    pub query: String,
    /// Number of events to emit before signalling end-of-stream.
    pub count: u32,
    /// Per-event delay in milliseconds. `None` = no sleep (fast
    /// tests); `Some(ms)` = paced output.
    pub delay_ms: Option<u64>,
}

impl Default for GoogleNewsConfig {
    fn default() -> Self {
        Self {
            query: "Bitcoin".into(),
            count: 5,
            delay_ms: None,
        }
    }
}

/// `google_news_search` Input Adapter. Emits `count` events whose
/// payload is `"<query>,<sequence>,<title>\n"` with the title
/// cycling through [`FAKE_TITLES`].
pub struct GoogleNewsInput {
    config: GoogleNewsConfig,
    emitted: u32,
    started_at_ms: u64,
}

impl InputAdapter for GoogleNewsInput {
    type Config = GoogleNewsConfig;

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

/// Factory for the google_news plugin. Holds no state; both
/// methods are pure.
pub struct GoogleNewsFactory;

impl Factory for GoogleNewsFactory {
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
        let vtable: *const InputAdapterVtable = &vtable_shim::VTABLE;
        let mut input_adapters = std::collections::HashMap::new();
        input_adapters.insert("search".to_string(), vtable);
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
            input_adapters,
            output_adapters: std::collections::HashMap::new(),
            handlers: std::collections::HashMap::new(),
        })
    }
}

mod vtable_shim {
    use std::sync::Mutex;

    use bee_adapter::InputAdapter;
    use bee_plugin_sdk::event::{encode_event, EventBytes};
    use bee_plugin_sdk::vtable::InputAdapterVtable;

    use super::{AdapterResult, GoogleNewsConfig, GoogleNewsInput, Event};

    pub struct Ctx {
        pub input: Mutex<GoogleNewsInput>,
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime")
            .block_on(f)
    }

    pub unsafe extern "C" fn open(
        config_ptr: *const u8,
        config_len: usize,
        _err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void {
        let bytes = std::slice::from_raw_parts(config_ptr, config_len);
        let cfg: GoogleNewsConfig = match bincode::deserialize(bytes) {
            Ok(c) => c,
            Err(_) => return std::ptr::null_mut(),
        };
        let input = match block_on(GoogleNewsInput::open(cfg)) {
            Ok(i) => i,
            Err(_) => return std::ptr::null_mut(),
        };
        let ctx = Box::new(Ctx {
            input: Mutex::new(input),
        });
        Box::into_raw(ctx) as *mut std::ffi::c_void
    }

    pub unsafe extern "C" fn next(
        ctx: *mut std::ffi::c_void,
        out: *mut EventBytes,
    ) -> i32 {
        let result: AdapterResult<Option<Event>> = block_on(async move {
            let ctx = &*(ctx as *const Ctx);
            let mut input = ctx.input.lock().unwrap();
            input.next().await
        });
        match result {
            Ok(Some(event)) => {
                let bytes = encode_event(&event);
                let len = bytes.len();
                let ptr = bytes.as_ptr();
                std::mem::forget(bytes);
                *out = EventBytes { ptr, len };
                1
            }
            Ok(None) => {
                *out = EventBytes::EMPTY;
                0
            }
            Err(_) => -1,
        }
    }

    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let ctx = Box::from_raw(ctx as *mut Ctx);
        let input = ctx.input.into_inner().expect("mutex poisoned");
        let _ = block_on(input.close());
        0
    }

    pub const VTABLE: InputAdapterVtable = InputAdapterVtable {
        open,
        next,
        close,
    };
}

bee_plugin_sdk::cdylib_plugin!(GoogleNewsFactory);

#[cfg(test)]
mod tests {
    use super::*;
    use bee_plugin_sdk::event::EventBytes;

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
        let config = GoogleNewsConfig {
            query: "Ethereum".into(),
            count: 3,
            ..Default::default()
        };
        let events = collect_events::<_, GoogleNewsInput>(config)
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
        let events = collect_events::<_, GoogleNewsInput>(
            GoogleNewsConfig::default(),
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
        let config = GoogleNewsConfig {
            count: 0,
            ..Default::default()
        };
        let events = collect_events::<_, GoogleNewsInput>(config)
            .await
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn factory_manifest_declares_search_adapter() {
        let m = GoogleNewsFactory::manifest();
        assert_eq!(m.name.0, "google_news");
        assert_eq!(m.abi_version, "v1");
        assert_eq!(m.adapters.len(), 1);
        assert_eq!(m.adapters[0].name, "search");
        assert!(m.adapters[0].is_input);
    }

    #[test]
    fn factory_init_returns_handle_with_manifest() {
        let h = GoogleNewsFactory::init().unwrap();
        assert_eq!(h.manifest.name.0, "google_news");
    }

    #[test]
    fn vtable_open_next_close_round_trips_news_event() {
        let handle = GoogleNewsFactory::init().expect("init");
        let vtable = *handle
            .input_adapters
            .get("search")
            .expect("search vtable");
        let cfg = GoogleNewsConfig {
            query: "Ethereum".into(),
            count: 2,
            ..Default::default()
        };
        let cfg_bytes = bincode::serialize(&cfg).unwrap();
        let ctx = unsafe {
            ((*vtable).open)(cfg_bytes.as_ptr(), cfg_bytes.len(), std::ptr::null_mut())
        };
        assert!(!ctx.is_null(), "open returned null");
        for expected_seq in 0..2 {
            let mut out = EventBytes::EMPTY;
            let rc = unsafe { ((*vtable).next)(ctx, &mut out) };
            assert_eq!(rc, 1, "event {expected_seq}");
            let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
            let event: Event = bincode::deserialize(bytes).expect("decode");
            assert_eq!(event.sequence, expected_seq as u64);
            let payload = String::from_utf8_lossy(&event.payload);
            let prefix = format!("Ethereum,{},", expected_seq);
            assert!(
                payload.starts_with(&prefix),
                "event {expected_seq} payload: {payload}"
            );
        }
        let mut out = EventBytes::EMPTY;
        let rc = unsafe { ((*vtable).next)(ctx, &mut out) };
        assert_eq!(rc, 0, "expected end-of-stream, got {rc}");
        unsafe { ((*vtable).close)(ctx) };
    }

    #[test]
    fn vtable_open_with_garbage_config_returns_null() {
        let handle = GoogleNewsFactory::init().expect("init");
        let vtable = *handle
            .input_adapters
            .get("search")
            .expect("search vtable");
        let garbage = vec![0xFFu8; 8];
        let ctx = unsafe {
            ((*vtable).open)(garbage.as_ptr(), garbage.len(), std::ptr::null_mut())
        };
        assert!(ctx.is_null(), "open with garbage should return null");
    }
}
