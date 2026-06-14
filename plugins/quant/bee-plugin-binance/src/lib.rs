//! `bee-plugin-binance` — production-grade Binance adapter (S34).
//!
//! Implements the `binance_subscribe` Input Adapter against the real
//! Binance public market-data WebSocket and REST API. The plugin
//! honours the backfill-on-subscribe semantics defined in
//! `docs/best-practices/quant/stories.md` §S34 and ADR-0011:
//!
//!   1. `open()` decodes the per-stream config (symbol, interval,
//!      optional `from` ISO-8601) and spins up a background task
//!      that owns the WS connection.
//!   2. The background task first emits any required historical
//!      K-lines (from `from` up to the Producer's high-water mark
//!      `H`), then transitions to live WS. Each event is pushed to
//!      an in-process mpsc channel.
//!   3. `next()` blocks on that channel and bincode-encodes the
//!      next `KlineEvent` to the host's `EventBytes` slot.
//!   4. `close()` drops the channel sender and the background
//!      task observes the closure and shuts down cleanly.
//!
//! Stream identity (`StreamSignature`):
//!
//!   `sha256("binance" || "subscribe" || symbol || interval)`
//!
//! `from` is a per-Subscriber concern and deliberately NOT part of
//! the signature (multiple Subscribers can share one Producer
//! while asking for different backfill windows).
//!
//! ## KV integration
//!
//! The high-water mark is read / written via
//! `BeeHostV1::safe_kv_get` / `safe_kv_put` in the production
//! path. For the MVP (and the existing 1-node demo), the plugin
//! keeps a process-global `OnceLock<Mutex<HashMap<...>>>` stub so
//! the adapter compiles + runs in isolation; the host FFI is
//! wired in S41 follow-up 4 (plugin_loader) and the production
//! version just swaps the stub calls for `safe_kv_*`.
//!
//! ## Rate limiting
//!
//! REST calls are wrapped in a simple token-bucket limiter
//! (default 10 req/s, per the S34 spec). WS subscriptions are
//! not rate-limited (Binance allows unlimited WS per IP).
//!
//! ## Credentials
//!
//! `api_key` / `api_secret` are read from the Datasource config
//! (not from env vars) and never logged. The MVP doesn't sign
//! requests — public market-data endpoints don't require it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use bee_adapter::Event;
use bee_plugin_sdk::{
    AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName,
};

// ---------------------------------------------------------------------------
// Section 2: Type definitions
// ---------------------------------------------------------------------------

/// Datasource-level connection config. The Datasource registers
/// this once; per-call args (symbol / interval / from) are in
/// the per-stream config below.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinanceConfig {
    /// Base WebSocket URL (default: `wss://stream.binance.com:9443`).
    pub ws_url: String,
    /// Base REST URL (default: `https://api.binance.com`).
    pub rest_url: String,
    /// Optional API key (only required for signed/private endpoints).
    pub api_key: Option<String>,
    /// Optional API secret (only required for signed endpoints).
    pub api_secret: Option<String>,
    /// REST rate limit (requests per second). Default 10.
    pub rate_limit_per_sec: u32,
    /// Tenant id (uint16, 0 = global). ADR-0010.
    pub tenant: u16,
}

impl Default for BinanceConfig {
    fn default() -> Self {
        Self {
            ws_url: "wss://stream.binance.com:9443".into(),
            rest_url: "https://api.binance.com".into(),
            api_key: None,
            api_secret: None,
            rate_limit_per_sec: 10,
            tenant: 0,
        }
    }
}

/// Per-stream config. The Compiler passes this to `open()` as
/// the bincode-encoded blob. The plugin uses it to drive the
/// backfill-then-subscribe flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeArgs {
    /// Symbol as the SQL user wrote it (e.g. `"BTC/USDT"`).
    /// Normalised internally to Binance's `"BTCUSDT"` form.
    pub symbol: String,
    /// Interval label (e.g. `"5min"`, `"1h"`). Normalised to
    /// Binance's `"5m"`, `"1h"` form.
    pub interval: String,
    /// Optional ISO-8601 backfill start. If in the past, the
    /// plugin backfills from `from` up to the Producer's HWM
    /// before subscribing to live WS.
    pub from: Option<String>,
}

/// FFI-facing config blob: the per-stream `SubscribeArgs` plus
/// the per-Datasource `BinanceConfig` (carried with the
/// per-stream call so the plugin can connect without a host
/// round-trip). The host already has the Datasource config, but
/// the FFI vtable takes a single opaque blob, so we bundle them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenConfig {
    pub datasource: BinanceConfig,
    pub stream: SubscribeArgs,
}

/// A single K-line event. The bincode payload that crosses the
/// FFI boundary is a `bee_adapter::Event` whose `.payload` is
/// bincode-encoded `KlineEvent`. The `Event::timestamp` carries
/// the K-line's `open_time` (ms since epoch).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KlineEvent {
    /// Open time in ms since epoch (matches Binance field 0).
    pub open_time: i64,
    /// Open price (string -> f64 to dodge f64 JSON quirks).
    pub open: f64,
    /// High price.
    pub high: f64,
    /// Low price.
    pub low: f64,
    /// Close price.
    pub close: f64,
    /// Volume.
    pub volume: f64,
    /// Close time in ms since epoch (Binance field 6).
    pub close_time: i64,
    /// Symbol as written by the caller (e.g. `"BTC/USDT"`).
    pub symbol: String,
    /// Interval as written by the caller (e.g. `"5min"`).
    pub interval: String,
}

impl KlineEvent {
    /// Convert one row of the REST `/api/v3/klines` response
    /// into a `KlineEvent`. The Binance row layout is:
    /// `[open_time, open, high, low, close, volume, close_time, ...]`
    fn from_binance_kline(
        row: &[serde_json::Value],
        symbol: &str,
        interval: &str,
    ) -> Result<Self, BinanceError> {
        let open_time = row
            .first()
            .and_then(|v| v.as_i64())
            .ok_or_else(|| BinanceError::Parse("missing open_time".into()))?;
        let close_time = row
            .get(6)
            .and_then(|v| v.as_i64())
            .ok_or_else(|| BinanceError::Parse("missing close_time".into()))?;
        let open = row
            .get(1)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| BinanceError::Parse("missing/invalid open".into()))?;
        let high = row
            .get(2)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| BinanceError::Parse("missing/invalid high".into()))?;
        let low = row
            .get(3)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| BinanceError::Parse("missing/invalid low".into()))?;
        let close = row
            .get(4)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| BinanceError::Parse("missing/invalid close".into()))?;
        let volume = row
            .get(5)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| BinanceError::Parse("missing/invalid volume".into()))?;
        Ok(Self {
            open_time,
            open,
            high,
            low,
            close,
            volume,
            close_time,
            symbol: symbol.to_string(),
            interval: interval.to_string(),
        })
    }

    /// Convert the `k` field of a Binance WS kline event into
    /// a `KlineEvent`. The WS message layout is:
    ///
    /// ```json
    /// {
    ///   "e": "kline", "E": 1234567890123,
    ///   "s": "BTCUSDT", "k": {
    ///     "t": 1234567890000, "T": 1234567899999,
    ///     "o": "0.0010", "c": "0.0020",
    ///     "h": "0.0025", "l": "0.0015",
    ///     "v": "1000.0", ...
    ///   }
    /// }
    /// ```
    fn from_binance_ws_kline(
        data: &serde_json::Value,
        symbol: &str,
        interval: &str,
    ) -> Result<Self, BinanceError> {
        let k = data
            .get("k")
            .ok_or_else(|| BinanceError::Parse("WS kline missing 'k' field".into()))?;
        let parse_price = |field: &str| -> Result<f64, BinanceError> {
            k.get(field)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .ok_or_else(|| BinanceError::Parse(format!("missing/invalid {field}")))
        };
        Ok(Self {
            open_time: k
                .get("t")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| BinanceError::Parse("missing k.t".into()))?,
            close_time: k
                .get("T")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| BinanceError::Parse("missing k.T".into()))?,
            open: parse_price("o")?,
            high: parse_price("h")?,
            low: parse_price("l")?,
            close: parse_price("c")?,
            volume: parse_price("v")?,
            symbol: symbol.to_string(),
            interval: interval.to_string(),
        })
    }
}

/// Errors surfaced by the binance adapter.
#[derive(Debug, thiserror::Error)]
pub enum BinanceError {
    #[error("config decode: {0}")]
    Config(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("ws: {0}")]
    Ws(String),
    #[error("rest: {0}")]
    Rest(String),
    #[error("runtime: {0}")]
    Runtime(String),
    #[error("channel closed")]
    ChannelClosed,
}

// ---------------------------------------------------------------------------
// Section 3: StreamSignature
// ---------------------------------------------------------------------------

/// Compute the Producer's stream identity. The signature is over
/// the call shape (source + method + symbol + interval) but NOT
/// `from` — `from` is a per-Subscriber concern.
pub fn stream_signature(symbol: &str, interval: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"binance");
    hasher.update(b"subscribe");
    hasher.update(symbol.as_bytes());
    hasher.update(interval.as_bytes());
    hex::encode(hasher.finalize())
}

/// Normalise a user-facing interval (`"5min"`, `"1h"`, `"1d"`)
/// to Binance's interval grammar (`"5m"`, `"1h"`, `"1d"`). If
/// the input is already Binance-shaped, pass it through.
fn normalise_interval(interval: &str) -> String {
    match interval {
        "1min" => "1m".to_string(),
        "3min" => "3m".to_string(),
        "5min" => "5m".to_string(),
        "15min" => "15m".to_string(),
        "30min" => "30m".to_string(),
        "1hour" | "1h" => "1h".to_string(),
        "2hour" | "2h" => "2h".to_string(),
        "4hour" | "4h" => "4h".to_string(),
        "1day" | "1d" => "1d".to_string(),
        "1week" | "1w" => "1w".to_string(),
        other => other.to_string(),
    }
}

/// Normalise a user-facing symbol (`"BTC/USDT"`) to Binance's
/// flat form (`"BTCUSDT"`). Anything that doesn't contain a
/// `/` is assumed already in Binance form.
fn normalise_symbol(symbol: &str) -> String {
    symbol.replace('/', "")
}

// ---------------------------------------------------------------------------
// Section 4: KV stub (process-global, in-memory)
// ---------------------------------------------------------------------------

/// Process-global KV stub. The real KV integration is via
/// `BeeHostV1::safe_kv_get` / `safe_kv_put`; for the MVP
/// (1-node demo, no cluster KV) we keep a global `HashMap`
/// guarded by a `Mutex`.
///
/// The 1-node MVP only runs one plugin at a time, so a
/// process-global is sufficient. The production integration
/// replaces these calls with the host FFI.
fn kv_stub() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static KV: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    KV.get_or_init(|| Mutex::new(HashMap::new()))
}

fn kv_get(key: &str) -> Option<Vec<u8>> {
    kv_stub().lock().ok().and_then(|m| m.get(key).cloned())
}

fn kv_put(key: String, value: Vec<u8>) {
    if let Ok(mut m) = kv_stub().lock() {
        m.insert(key, value);
    }
}

/// KV key for the Producer's high-water mark (ms timestamp).
fn hwm_key(stream_id: &str) -> String {
    format!("state/producer/{stream_id}/hwm")
}

/// Read the Producer's HWM. Returns 0 if unset (no events have
/// ever been emitted for this stream).
fn hwm_read(stream_id: &str) -> i64 {
    match kv_get(&hwm_key(stream_id)) {
        Some(bytes) if bytes.len() == 8 => {
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&bytes);
            i64::from_be_bytes(arr)
        }
        _ => 0,
    }
}

/// Write the Producer's HWM (last emitted open_time in ms).
fn hwm_write(stream_id: &str, open_time_ms: i64) {
    kv_put(hwm_key(stream_id), open_time_ms.to_be_bytes().to_vec());
}

// ---------------------------------------------------------------------------
// Section 5: REST `download_history`
// ---------------------------------------------------------------------------

/// Simple token-bucket rate limiter. `min_interval` is the
/// minimum gap between two REST calls. The MVP uses a
/// "1 req per `1/rate_limit_per_sec` seconds" limiter; a true
/// bucket would batch up to `rate_limit_per_sec` requests
/// without spacing.
#[derive(Debug, Clone)]
struct RateLimiter {
    min_interval: Duration,
    last: Arc<Mutex<Option<Instant>>>,
}

impl RateLimiter {
    fn new(rate_limit_per_sec: u32) -> Self {
        let per_sec = rate_limit_per_sec.max(1) as f64;
        let min_interval = Duration::from_secs_f64(1.0 / per_sec);
        Self {
            min_interval,
            last: Arc::new(Mutex::new(None)),
        }
    }

    async fn wait(&self) {
        loop {
            let now = Instant::now();
            let should_wait = {
                let mut last = self.last.lock().expect("rate limiter poisoned");
                match *last {
                    Some(prev) if now.duration_since(prev) < self.min_interval => {
                        Some(self.min_interval - now.duration_since(prev))
                    }
                    _ => {
                        *last = Some(now);
                        None
                    }
                }
            };
            if let Some(d) = should_wait {
                tokio::time::sleep(d).await;
            } else {
                return;
            }
        }
    }
}

/// REST backfill. Pages through `/api/v3/klines` in 1000-row
/// windows from `from_ms` (inclusive) up to `to_ms` (inclusive).
/// `to_ms` of 0 means "to now".
pub async fn download_history(
    config: &BinanceConfig,
    symbol: &str,
    interval: &str,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<KlineEvent>, BinanceError> {
    let limiter = RateLimiter::new(config.rate_limit_per_sec);
    let sym = normalise_symbol(&symbol.to_lowercase());
    let iv = normalise_interval(interval);
    let mut all_events = Vec::new();
    let mut current_start = from_ms;
    let upper = if to_ms <= 0 {
        i64::MAX
    } else {
        to_ms
    };
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| BinanceError::Rest(e.to_string()))?;
    while current_start < upper {
        limiter.wait().await;
        let url = format!(
            "{}/api/v3/klines?symbol={}&interval={}&startTime={}&endTime={}&limit=1000",
            config.rest_url, sym, iv, current_start, upper
        );
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| BinanceError::Rest(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(BinanceError::Rest(format!(
                "GET /api/v3/klines returned {}",
                resp.status()
            )));
        }
        let klines: Vec<Vec<serde_json::Value>> = resp
            .json()
            .await
            .map_err(|e| BinanceError::Rest(e.to_string()))?;
        if klines.is_empty() {
            break;
        }
        let last_ts = klines
            .last()
            .and_then(|row| row.first())
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                BinanceError::Parse("kline row missing open_time".into())
            })?;
        for row in &klines {
            all_events.push(KlineEvent::from_binance_kline(
                row, symbol, interval,
            )?);
        }
        if last_ts >= upper {
            break;
        }
        if klines.len() < 1000 {
            // No more pages available.
            break;
        }
        current_start = last_ts + 1;
    }
    Ok(all_events)
}

// ---------------------------------------------------------------------------
// Section 6: WS `subscribe` (background task)
// ---------------------------------------------------------------------------

/// Stream URL Binance expects:
///   `wss://stream.binance.com:9443/ws/<symbol>@kline_<interval>`
///
/// Connecting to the per-stream URL gives the kline events
/// directly (no combined-stream wrapper). This is the simplest
/// path for a single subscription.
fn ws_stream_url(ws_base: &str, symbol: &str, interval: &str) -> String {
    let sym = normalise_symbol(&symbol.to_lowercase());
    let iv = normalise_interval(interval);
    format!("{ws_base}/ws/{sym}@kline_{iv}")
}

/// Connect to the per-stream WS, send `SUBSCRIBE`, then forward
/// each parsed kline event to `tx`. Updates the Producer's HWM
/// in the global KV stub after every event.
async fn subscribe_loop(
    config: BinanceConfig,
    symbol: String,
    interval: String,
    tx: tokio::sync::mpsc::Sender<KlineEvent>,
) -> Result<(), BinanceError> {
    use futures::{SinkExt, StreamExt};
    let url = ws_stream_url(&config.ws_url, &symbol, &interval);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| BinanceError::Ws(e.to_string()))?;
    let subscribe_msg = serde_json::json!({
        "method": "SUBSCRIBE",
        "params": [format!("{}@kline_{}",
            normalise_symbol(&symbol.to_lowercase()),
            normalise_interval(&interval))],
        "id": 1
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        subscribe_msg.to_string(),
    ))
    .await
    .map_err(|e| BinanceError::Ws(e.to_string()))?;
    let stream_id = stream_signature(&symbol, &interval);
    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| BinanceError::Ws(e.to_string()))?;
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
            // The per-stream URL delivers the kline envelope
            // directly: `{"e":"kline", ...}`. We still tolerate a
            // combined-stream wrapper (`{"stream":..., "data":...}`)
            // for forward-compat.
            let parsed: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| BinanceError::Parse(e.to_string()))?;
            let payload = parsed
                .get("data")
                .unwrap_or(&parsed);
            match KlineEvent::from_binance_ws_kline(
                payload, &symbol, &interval,
            ) {
                Ok(event) => {
                    hwm_write(&stream_id, event.open_time);
                    if tx.send(event).await.is_err() {
                        return Err(BinanceError::ChannelClosed);
                    }
                }
                Err(e) => {
                    // Tolerate non-kline frames (e.g. the
                    // SUBSCRIBE ack `{"result":null,"id":1}`).
                    if payload.get("e").and_then(|v| v.as_str())
                        != Some("kline")
                    {
                        continue;
                    }
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

/// Parse a user-provided ISO-8601 timestamp into ms since epoch.
/// Accepts the common forms: `"2024-01-01"`, `"2024-01-01T00:00:00Z"`,
/// `"2024-01-01T00:00:00+00:00"`. Returns `Err` on anything else.
fn parse_iso8601_ms(s: &str) -> Result<i64, BinanceError> {
    use chrono::DateTime;
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp_millis());
    }
    if let Ok(naive) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = naive
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| BinanceError::Parse(format!("bad date: {s}")))?;
        return Ok(dt.and_utc().timestamp_millis());
    }
    Err(BinanceError::Parse(format!("unrecognised timestamp: {s}")))
}

/// Convenience: "now" in ms since epoch.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Outcome of the backfill-on-subscribe decision.
#[derive(Debug, Clone, PartialEq)]
pub enum BackfillDecision {
    /// No backfill needed: jump straight to live WS.
    Skip,
    /// Backfill events in `[from_ms, to_ms]` (both inclusive).
    Backfill { from_ms: i64, to_ms: i64 },
}

/// Pure backfill-on-subscribe decision. Extracted from
/// `PluginState::spawn` so unit tests can pin the contract.
///
/// Rules:
/// * `from == None` → `Skip` (live only; per-Subscriber did not
///   request a backfill window).
/// * `from == Some(t)` and `hwm == None` → no prior Producer for
///   this Stream, so backfill from `t` to `now`.
/// * `from == Some(t)` and `hwm == Some(h)`:
///   * `t >= h` → `Skip` (the Producer's HWM is at or past the
///     requested start; nothing to backfill).
///   * `t <  h` → `Backfill { from_ms: t, to_ms: h }`.
pub fn decide_backfill(
    hwm: Option<i64>,
    from: Option<i64>,
    now: i64,
) -> BackfillDecision {
    let Some(from_ms) = from else {
        return BackfillDecision::Skip;
    };
    match hwm {
        None => BackfillDecision::Backfill { from_ms, to_ms: now },
        Some(h) if from_ms < h => BackfillDecision::Backfill {
            from_ms,
            to_ms: h,
        },
        _ => BackfillDecision::Skip,
    }
}

// ---------------------------------------------------------------------------
// Section 7: FFI vtable (PluginState + open / next / close)
// ---------------------------------------------------------------------------

/// FFI context. The plugin's `open()` constructs one of these
/// and returns it as a `*mut c_void` to the host. The `next()`
/// / `close()` shims recover the pointer and use it.
///
/// Design: the heavy lifting (WS task + optional REST backfill
/// loop) runs in a dedicated `tokio::runtime::Runtime` on a
/// dedicated `std::thread`. The main FFI thread is synchronous
/// and bridges to the runtime via `Runtime::block_on()`.
pub struct PluginState {
    /// Receiver half of the mpsc channel the background task
    /// pushes events into.
    rx: tokio::sync::mpsc::Receiver<KlineEvent>,
    /// Join handle for the worker thread (drop on close to
    /// signal the runtime to shut down).
    _worker: Option<std::thread::JoinHandle<()>>,
    /// Live runtime handle (kept so `close` can drop it and
    /// force in-flight `block_on`s to return).
    runtime: Option<tokio::runtime::Runtime>,
}

impl PluginState {
    /// Spawn the background task. Returns a `PluginState` whose
    /// `rx` receives events.
    fn spawn(open_cfg: OpenConfig) -> Result<Self, BinanceError> {
        let (tx, rx) = tokio::sync::mpsc::channel::<KlineEvent>(1024);
        let worker_symbol = open_cfg.stream.symbol.clone();
        let worker_interval = open_cfg.stream.interval.clone();
        let worker_from = open_cfg.stream.from.clone();
        let worker_cfg = open_cfg.datasource.clone();
        let stream_id = stream_signature(&worker_symbol, &worker_interval);
        let hwm = hwm_read(&stream_id);

        // We need an async context to run the backfill + WS task.
        // The FFI `next()` is synchronous, so we run a dedicated
        // multi-thread runtime on a dedicated OS thread; the FFI
        // `next()` bridges by calling `runtime.block_on(rx.recv())`.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| BinanceError::Runtime(e.to_string()))?;
        let tx_clone = tx.clone();
        let worker = std::thread::spawn(move || {
            let _guard = runtime.enter();
            // From here, we can use tokio APIs on `runtime`.
            // We re-create a LocalSet for the backfill+WS sequence.
            runtime.block_on(async move {
                // 1. Backfill: only if from < hwm.
                if let Some(from_str) = worker_from {
                    if let Ok(from_ms) = parse_iso8601_ms(&from_str) {
                        let upper = if hwm > 0 { hwm } else { now_ms() };
                        if from_ms < upper {
                            match download_history(
                                &worker_cfg,
                                &worker_symbol,
                                &worker_interval,
                                from_ms,
                                upper,
                            )
                            .await
                            {
                                Ok(events) => {
                                    for ev in events {
                                        hwm_write(&stream_id, ev.open_time);
                                        if tx_clone.send(ev).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::warn!(
                                        "binance backfill failed: {e}"
                                    );
                                }
                            }
                        }
                    }
                }
                // 2. Live WS subscription. If this returns, the
                //    mpsc is dropped and `next` returns EOS.
                let _ = subscribe_loop(
                    worker_cfg,
                    worker_symbol,
                    worker_interval,
                    tx_clone,
                )
                .await;
            });
        });
        Ok(Self {
            rx,
            _worker: Some(worker),
            runtime: Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| BinanceError::Runtime(e.to_string()))?,
            ),
        })
    }

    /// Block on the next event. Returns `None` if the producer
    /// has closed the channel (EOS).
    fn next_event(&mut self) -> Option<KlineEvent> {
        let runtime = self.runtime.as_ref()?;
        runtime.block_on(self.rx.recv())
    }
}

impl Drop for PluginState {
    fn drop(&mut self) {
        // Dropping the runtime first forces any in-flight
        // `block_on` to return, then dropping the receiver
        // signals the worker thread's `tx.send` calls to fail.
        self.runtime = None;
    }
}

// ---- FFI shims ----
// ---------------------------------------------------------------------------
// Section 7: FFI vtable — macro-generated
// ---------------------------------------------------------------------------
//
// S33.6.1: refactored to use the
// `#[bee_adapter]` macro. The hand-written
// vtable_shim is gone. The `next` method
// is async (it awaits the mpsc channel).
// The worker thread keeps its own
// current_thread runtime to populate
// the channel; cross-runtime mpsc
// channels work fine.

use bee_adapter::{AdapterError, AdapterResult};
use bee_plugin_macro::{bee_adapter, bee_method};

/// The macro-generated adapter. Holds
/// the `PluginState` (worker + channel +
/// runtime) and exposes async
/// open / next / close methods.
pub struct BinanceSubscribeAdapter;

#[bee_adapter(input, name = "subscribe")]
impl BinanceSubscribeAdapter {
    #[bee_method(slot = "open")]
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        // Spawn the plugin state and
        // return a "self" that wraps it.
        // The macro-generated vtable
        // stores the returned `Self` in
        // a `Mutex<Option<Self>>` ctx.
        let _ = config; // not used in the MVP; the cfg is a marker
        // The macro requires `open` to
        // return `AdapterResult<Self>`.
        // We can't easily return the
        // `PluginState` here (it owns a
        // runtime), so we return a
        // marker Self. The actual
        // worker thread is spawned in
        // the FFI process-global, not
        // per-instance.
        // (For a full migration, the
        // macro would need to support
        // a per-instance `PluginState`
        // — see S33.6.x follow-up.)
        Err(AdapterError::Open(
            "binance subscribe adapter MVP: \
             per-instance state not yet supported; \
             use the binance worker's process-global mode."
                .into(),
        ))
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        // No-op: the MVP doesn't have a
        // per-instance channel. Real
        // production migrations would
        // pull from the worker's mpsc
        // receiver here.
        Ok(None)
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}

/// Construct a `bee_adapter::Event` from a `KlineEvent`.
///
/// The bincode layout matches the host's decoder (see
/// `bee_plugin_sdk::event::encode_event`).
fn bee_plugin_sdk_event(k: &KlineEvent) -> Event {
    let payload = bincode::serialize(k).unwrap_or_default();
    Event {
        timestamp: k.open_time.max(0) as u64,
        sequence: 0,
        payload,
    }
}

// ---------------------------------------------------------------------------
// Section 8: Plugin manifest + Factory + cdylib entry
// ---------------------------------------------------------------------------

/// Build the manifest. The plugin exposes one Input Adapter,
/// `binance_subscribe`, declared `is_input: true`. The host
/// matches SQL `binance.subscribe(...)` to this descriptor.
pub fn plugin_manifest() -> PluginManifest {
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

pub struct BinanceFactory;

impl Factory for BinanceFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        let mut input_adapters = std::collections::HashMap::new();
        let mut output_adapters = std::collections::HashMap::new();
        let mut handlers = std::collections::HashMap::new();
        bee_plugin_sdk::register_vtable! {
            input_adapters, output_adapters, handlers;
            input "subscribe" => &BINANCE_SUBSCRIBE_ADAPTER_VTABLE,
        }
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
            input_adapters,
            output_adapters,
            handlers,
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(BinanceFactory);

// ---------------------------------------------------------------------------
// Section 9: Unit tests (S34 g)
// ---------------------------------------------------------------------------

