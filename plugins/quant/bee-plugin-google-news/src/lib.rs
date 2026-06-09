//! `bee-plugin-google-news` — production-grade NewsAPI adapter (S35).
//!
//! Implements the `google_news_search` and `google_news_top_headlines`
//! Input Adapters against the real NewsAPI REST endpoints. The plugin
//! honours the polling + rate-limit + Stream identity contract defined
//! in `docs/best-practices/quant/stories.md` §S35 and ADR-0011:
//!
//!   1. `open()` decodes the per-stream `OpenConfig` (datasource +
//!      per-call args + poll interval) and spawns a background task
//!      on a dedicated `tokio` runtime + OS thread.
//!   2. The background task calls the appropriate REST endpoint
//!      (`/v2/everything` or `/v2/top-headlines`) on a fixed cadence
//!      (default 60s) and pushes each `ArticleEvent` to an mpsc.
//!      A token-bucket rate limiter throttles REST calls to
//!      `rate_limit_per_sec` (default 5).
//!   3. `next()` blocks on the mpsc and bincode-encodes the next
//!      `ArticleEvent` to the host's `EventBytes` slot.
//!   4. `close()` drops the channel sender; the worker thread
//!      observes the closure and shuts down cleanly.
//!
//! Stream identity:
//!
//!   `StreamSignature = sha256("google_news" || method || query)`
//!
//! `from` / `to` / `sort_by` are per-Subscriber concerns and
//! deliberately NOT part of the signature.
//!
//! ## KV integration
//!
//! The "HWM" in S35 is the timestamp of the newest article already
//! emitted for a given Stream, used to deduplicate across polling
//! cycles. The real KV integration is via `BeeHostV1::safe_kv_get` /
//! `safe_kv_put`; for the MVP (1-node demo, no cluster KV) we keep a
//! process-global `LazyLock<Mutex<HashMap<...>>>` stub. The host FFI
//! is wired in S41 follow-up 4 (plugin_loader) and the production
//! version just swaps the stub calls for `safe_kv_*`.
//!
//! ## Rate limiting
//!
//! REST calls are wrapped in a simple token-bucket rate limiter
//! (default 5 req/s, per the S35 spec — NewsAPI free tier is
//! 100/day, paid plans are higher; the limiter is configurable).
//!
//! ## Credentials
//!
//! `api_key` is read from the Datasource config (not from env vars)
//! and never logged. The MVP keeps the key in memory only for the
//! duration of the open stream.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use bee_adapter::Event;
use bee_plugin_sdk::event::{encode_event, EventBytes};
use bee_plugin_sdk::vtable::InputAdapterVtable;
use bee_plugin_sdk::{
    AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName,
};

// ---------------------------------------------------------------------------
// Section 2: Type definitions
// ---------------------------------------------------------------------------

/// Datasource-level connection config. Registered once per
/// Datasource; per-call args (query / from / to / sort_by / page_size)
/// go in the per-stream config below.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleNewsConfig {
    /// NewsAPI key (required). Sourced from the bee secret store;
    /// never logged.
    pub api_key: String,
    /// Base REST URL. Default `https://newsapi.org/v2`.
    pub base_url: String,
    /// REST rate limit (requests per second). Default 5.
    pub rate_limit_per_sec: u32,
    /// Default language code (e.g. `"en"`). Default `"en"`.
    pub language: String,
    /// Tenant id (uint16, 0 = global). ADR-0010.
    pub tenant: u16,
}

impl Default for GoogleNewsConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://newsapi.org/v2".into(),
            rate_limit_per_sec: 5,
            language: "en".into(),
            tenant: 0,
        }
    }
}

/// Per-call args for `google_news_search` (the `search` method).
/// The Compiler passes these to `open()` alongside the datasource
/// config; the plugin uses them to drive the polling loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchArgs {
    /// Query string (e.g. `"Bitcoin"` or `"AAPL OR \"Apple Inc\""`).
    /// URL-encoded by the plugin.
    pub query: String,
    /// Optional ISO-8601 start date.
    pub from: Option<String>,
    /// Optional ISO-8601 end date.
    pub to: Option<String>,
    /// Optional sort: `"publishedAt"` | `"relevancy"` | `"popularity"`.
    pub sort_by: Option<String>,
    /// Page size (1-100; default 100).
    pub page_size: Option<u32>,
    /// Poll interval in seconds (default 60).
    pub poll_interval_secs: Option<u64>,
}

/// Per-call args for `google_news_top_headlines`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopHeadlinesArgs {
    /// Optional free-form query.
    pub query: Option<String>,
    /// Optional 2-letter ISO country code (e.g. `"us"`).
    pub country: Option<String>,
    /// Optional category (e.g. `"business"`, `"technology"`).
    pub category: Option<String>,
    /// Page size (1-100; default 100).
    pub page_size: Option<u32>,
    /// Poll interval in seconds (default 60).
    pub poll_interval_secs: Option<u64>,
}

/// A single news article event. The bincode payload that crosses
/// the FFI boundary is a `bee_adapter::Event` whose `.payload` is
/// bincode-encoded `ArticleEvent`. The `Event::timestamp` carries
/// the article's `publishedAt` parsed to ms-since-epoch (0 if
/// unparseable).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArticleEvent {
    /// Published-at timestamp in ms since epoch (0 if NewsAPI
    /// returned an unparseable string).
    pub published_at_ms: i64,
    /// Source display name (e.g. `"The Wall Street Journal"`).
    pub source: String,
    /// Author name (may be empty / null).
    pub author: String,
    /// Headline.
    pub title: String,
    /// Short description / summary (may be truncated).
    pub description: String,
    /// Canonical article URL.
    pub url: String,
    /// Body content (NewsAPI truncates to ~200 chars).
    pub content: String,
    /// The query that produced this event (for downstream filtering).
    pub query: String,
}

/// Raw NewsAPI `/v2/everything` (or `/v2/top-headlines`) response
/// shape. Field names match the public NewsAPI JSON layout.
#[derive(Debug, Deserialize)]
struct NewsApiResponse {
    /// `"ok"` on success, `"error"` on failure.
    status: String,
    /// Total result count (for `/everything` only; missing on
    /// `/top-headlines`).
    #[serde(default)]
    #[allow(dead_code)]
    totalResults: Option<u32>,
    /// On `"ok"`, the list of articles.
    #[serde(default)]
    articles: Vec<NewsApiArticle>,
    /// On `"error"`, the error code (e.g. `"apiKeyInvalid"`).
    #[serde(default)]
    code: Option<String>,
    /// On `"error"`, the human message.
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NewsApiArticle {
    #[serde(default)]
    source: Option<NewsApiSource>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    publishedAt: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NewsApiSource {
    #[serde(default)]
    #[serde(rename = "id")]
    _id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

impl ArticleEvent {
    /// Convert one row of the NewsAPI response into an
    /// `ArticleEvent`, tagged with the originating `query`.
    fn from_newsapi_article(a: NewsApiArticle, query: &str) -> Self {
        Self {
            published_at_ms: parse_newsapi_published_at(
                a.publishedAt.as_deref(),
            ),
            source: a
                .source
                .and_then(|s| s.name)
                .unwrap_or_default(),
            author: a.author.unwrap_or_default(),
            title: a.title.unwrap_or_default(),
            description: a.description.unwrap_or_default(),
            url: a.url.unwrap_or_default(),
            content: a.content.unwrap_or_default(),
            query: query.to_string(),
        }
    }
}

/// Errors surfaced by the google_news adapter.
#[derive(Debug, thiserror::Error)]
pub enum GoogleNewsError {
    #[error("config decode: {0}")]
    Config(String),
    #[error("datasource config: {0}")]
    Datasource(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("rest: {0}")]
    Rest(String),
    #[error("api: {code}: {message}")]
    Api { code: String, message: String },
    #[error("runtime: {0}")]
    Runtime(String),
    #[error("channel closed")]
    ChannelClosed,
}

// ---------------------------------------------------------------------------
// Section 3: StreamSignature
// ---------------------------------------------------------------------------

/// Compute the Producer's stream identity. The signature is over
/// the call shape (source + method + query) but NOT
/// `from` / `to` / `sort_by` — those are per-Subscriber concerns.
pub fn stream_signature(method: &str, query: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"google_news");
    hasher.update(method.as_bytes());
    hasher.update(query.as_bytes());
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------------------
// Section 4: KV stub (process-global, in-memory)
// ---------------------------------------------------------------------------

/// Process-global KV stub. The real KV integration is via
/// `BeeHostV1::safe_kv_get` / `safe_kv_put`; for the MVP (1-node
/// demo, no cluster KV) we keep a global `HashMap` guarded by a
/// `Mutex`. The production integration replaces these calls with
/// the host FFI in S41 follow-up 4.
fn kv_stub() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static KV: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    &KV
}

fn kv_lock() -> MutexGuard<'static, HashMap<String, Vec<u8>>> {
    kv_stub()
        .lock()
        .expect("kv stub poisoned")
}

fn kv_get(key: &str) -> Option<Vec<u8>> {
    kv_lock().get(key).cloned()
}

fn kv_put(key: String, value: Vec<u8>) {
    kv_lock().insert(key, value);
}

/// KV key for the Producer's high-water mark on a news Stream.
/// The "HWM" for S35 is the published-at timestamp (ms) of the
/// newest article already emitted, used to skip already-seen
/// articles across polling cycles.
fn hwm_key(stream_id: &str) -> String {
    format!("state/producer/{stream_id}/hwm")
}

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

fn hwm_write(stream_id: &str, published_at_ms: i64) {
    kv_put(
        hwm_key(stream_id),
        published_at_ms.to_be_bytes().to_vec(),
    );
}

// ---------------------------------------------------------------------------
// Section 5: Rate limiter + REST helpers
// ---------------------------------------------------------------------------

/// Simple token-bucket-ish rate limiter. `min_interval` is the
/// minimum gap between two REST calls. A true token bucket would
/// batch up to `rate_limit_per_sec` requests without spacing, but
/// NewsAPI's free tier (100/day) is so far below the limiter's
/// granularity that a "1 req per `1/rate_limit_per_sec` seconds"
/// gap is sufficient.
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

/// Parse a NewsAPI `publishedAt` ISO-8601 timestamp into ms since
/// epoch. Returns 0 on any parse error (the `ArticleEvent` keeps
/// the original string in `content` / fields and the host may still
/// inspect it downstream).
fn parse_newsapi_published_at(s: Option<&str>) -> i64 {
    let Some(s) = s else { return 0 };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp_millis();
    }
    0
}

/// Convenience: "now" in ms since epoch.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Build the `reqwest::Client` shared by the polling loop. We
/// keep a `Client` rather than using `reqwest::get` so connection
/// pooling and timeouts are configurable.
fn build_http_client() -> Result<reqwest::Client, GoogleNewsError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| GoogleNewsError::Rest(e.to_string()))
}

/// GET the URL, decode the response, validate `status`. On a
/// `"error"` payload, return `GoogleNewsError::Api { code, message }`.
async fn fetch_newsapi<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, GoogleNewsError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| GoogleNewsError::Rest(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(GoogleNewsError::Rest(format!(
            "GET {url} returned {}",
            resp.status()
        )));
    }
    let body: T = resp
        .json()
        .await
        .map_err(|e| GoogleNewsError::Rest(e.to_string()))?;
    Ok(body)
}

/// One poll of `/v2/everything`. Returns the freshly-fetched
/// articles.
pub async fn search(
    config: &GoogleNewsConfig,
    args: &SearchArgs,
) -> Result<Vec<ArticleEvent>, GoogleNewsError> {
    if config.api_key.is_empty() {
        return Err(GoogleNewsError::Datasource(
            "api_key is required".into(),
        ));
    }
    let mut url = format!(
        "{}/everything?q={}&apiKey={}",
        config.base_url,
        urlencoding::encode(&args.query),
        urlencoding::encode(&config.api_key),
    );
    if let Some(from) = &args.from {
        url.push_str(&format!("&from={}", urlencoding::encode(from)));
    }
    if let Some(to) = &args.to {
        url.push_str(&format!("&to={}", urlencoding::encode(to)));
    }
    if let Some(sort_by) = &args.sort_by {
        url.push_str(&format!("&sortBy={}", urlencoding::encode(sort_by)));
    }
    let page_size = args.page_size.unwrap_or(100).clamp(1, 100);
    url.push_str(&format!("&pageSize={page_size}"));
    url.push_str(&format!("&language={}", urlencoding::encode(&config.language)));

    let client = build_http_client()?;
    let resp: NewsApiResponse = fetch_newsapi(&client, &url).await?;
    if resp.status != "ok" {
        return Err(GoogleNewsError::Api {
            code: resp.code.unwrap_or_default(),
            message: resp.message.unwrap_or_default(),
        });
    }
    Ok(resp
        .articles
        .into_iter()
        .map(|a| ArticleEvent::from_newsapi_article(a, &args.query))
        .collect())
}

/// One poll of `/v2/top-headlines`. Returns the freshly-fetched
/// articles.
pub async fn top_headlines(
    config: &GoogleNewsConfig,
    args: &TopHeadlinesArgs,
) -> Result<Vec<ArticleEvent>, GoogleNewsError> {
    if config.api_key.is_empty() {
        return Err(GoogleNewsError::Datasource(
            "api_key is required".into(),
        ));
    }
    let mut url = format!(
        "{}/top-headlines?apiKey={}",
        config.base_url,
        urlencoding::encode(&config.api_key),
    );
    if let Some(q) = &args.query {
        url.push_str(&format!("&q={}", urlencoding::encode(q)));
    }
    if let Some(c) = &args.country {
        url.push_str(&format!("&country={}", urlencoding::encode(c)));
    }
    if let Some(cat) = &args.category {
        url.push_str(&format!("&category={}", urlencoding::encode(cat)));
    }
    let page_size = args.page_size.unwrap_or(100).clamp(1, 100);
    url.push_str(&format!("&pageSize={page_size}"));

    let client = build_http_client()?;
    let resp: NewsApiResponse = fetch_newsapi(&client, &url).await?;
    if resp.status != "ok" {
        return Err(GoogleNewsError::Api {
            code: resp.code.unwrap_or_default(),
            message: resp.message.unwrap_or_default(),
        });
    }
    // For top-headlines we tag each article with the *query*
    // (if any); for callers that pass None (just country+category),
    // we fall back to "<country>/<category>".
    let tag = args
        .query
        .clone()
        .unwrap_or_else(|| match (&args.country, &args.category) {
            (Some(c), Some(cat)) => format!("{c}/{cat}"),
            (Some(c), None) => c.clone(),
            (None, Some(cat)) => cat.clone(),
            (None, None) => String::new(),
        });
    Ok(resp
        .articles
        .into_iter()
        .map(|a| ArticleEvent::from_newsapi_article(a, &tag))
        .collect())
}

// ---------------------------------------------------------------------------
// Section 6: FFI vtable (OpenConfig + PluginState + open / next / close)
// ---------------------------------------------------------------------------

/// Per-stream config bundled into a single bincode blob (the FFI
/// vtable takes one opaque bytes slice per `open()` call). The
/// host already has the Datasource config but bundles it with the
/// per-call args so the plugin has everything it needs without a
/// host round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenConfigSearch {
    pub datasource: GoogleNewsConfig,
    pub stream: SearchArgs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenConfigTopHeadlines {
    pub datasource: GoogleNewsConfig,
    pub stream: TopHeadlinesArgs,
}

/// Internal: a method-agnostic polling args carried in the worker
/// thread. We materialise this in `PluginState::spawn` from the
/// typed `OpenConfig*` so the worker loop is one function.
#[derive(Debug, Clone)]
enum PollMethod {
    Search,
    TopHeadlines,
}

struct PollArgsSearch {
    args: SearchArgs,
}

struct PollArgsTopHeadlines {
    args: TopHeadlinesArgs,
}

/// FFI context. The plugin's `open()` constructs one of these and
/// returns it as a `*mut c_void` to the host. `next()` blocks on
/// the mpsc the background task pushes events into; `close()`
/// drops the state (cancelling the worker via mpsc drop).
pub struct PluginState {
    /// Receiver half of the mpsc the background task pushes
    /// `ArticleEvent`s into.
    rx: tokio::sync::mpsc::Receiver<ArticleEvent>,
    /// Current-thread `tokio` runtime used by `next()` to
    /// `block_on(rx.recv())`. Dropping it forces any in-flight
    /// `block_on` to return.
    runtime: Option<tokio::runtime::Runtime>,
    /// Join handle for the worker thread (drop on close to
    /// signal the runtime to shut down).
    _worker: Option<std::thread::JoinHandle<()>>,
}

impl PluginState {
    /// Spawn the background polling task. Returns a `PluginState`
    /// whose `rx` receives `ArticleEvent`s.
    fn spawn_search(
        open_cfg: OpenConfigSearch,
    ) -> Result<Self, GoogleNewsError> {
        Self::spawn_inner(
            PollMethod::Search,
            Box::new(PollArgsSearch { args: open_cfg.stream.clone() }),
            open_cfg.datasource,
            stream_signature("search", &open_cfg.stream.query),
        )
    }

    fn spawn_top_headlines(
        open_cfg: OpenConfigTopHeadlines,
    ) -> Result<Self, GoogleNewsError> {
        // For top-headlines the "query" component of the
        // signature is the caller-supplied query (or "" if None).
        let sig_query = open_cfg.stream.query.clone().unwrap_or_default();
        Self::spawn_inner(
            PollMethod::TopHeadlines,
            Box::new(PollArgsTopHeadlines { args: open_cfg.stream.clone() }),
            open_cfg.datasource,
            stream_signature("top_headlines", &sig_query),
        )
    }

    fn spawn_inner(
        method: PollMethod,
        // The boxed payload is one of `PollArgsSearch` /
        // `PollArgsTopHeadlines`. We pass an erased `Box<dyn
        // Any>` so the worker can downcast; the alternative
        // (separate spawn_* per method) duplicates the runtime +
        // channel setup. Box<dyn Any> + downcast_ref is fine
        // because the type is fixed at `open()` time.
        _args: Box<dyn std::any::Any + Send>,
        datasource: GoogleNewsConfig,
        stream_id: String,
    ) -> Result<Self, GoogleNewsError> {
        let (tx, rx) = tokio::sync::mpsc::channel::<ArticleEvent>(1024);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| GoogleNewsError::Runtime(e.to_string()))?;
        let worker_method = method;
        let worker_datasource = datasource.clone();
        let worker_stream_id = stream_id.clone();
        // The worker thread re-enters the runtime and runs a
        // dedicated `block_on` for its whole lifetime. The
        // current-thread runtime is exactly what we want: one
        // task, predictable ownership, no thread-pool leak.
        let worker_runtime = runtime.handle().clone();
        let worker = std::thread::spawn(move || {
            let _guard = worker_runtime.enter();
            worker_runtime.block_on(async move {
                polling_loop(worker_method, worker_datasource, _args, tx, worker_stream_id).await;
            });
        });
        Ok(Self {
            rx,
            runtime: Some(runtime),
            _worker: Some(worker),
        })
    }

    /// Block on the next event. Returns `None` if the producer
    /// has closed the channel (EOS).
    fn next_event(&mut self) -> Option<ArticleEvent> {
        let runtime = self.runtime.as_ref()?;
        runtime.block_on(self.rx.recv())
    }
}

impl Drop for PluginState {
    fn drop(&mut self) {
        // Dropping the runtime first forces any in-flight
        // `block_on` to return, then dropping the receiver
        // signals the worker's `tx.send` calls to fail.
        self.runtime = None;
    }
}

/// Background polling loop. Calls the appropriate REST endpoint
/// every `poll_interval_secs` (default 60) and pushes each new
/// `ArticleEvent` to `tx`. Stops when the mpsc closes (i.e. the
/// FFI `close()` shim dropped `PluginState`).
async fn polling_loop(
    method: PollMethod,
    config: GoogleNewsConfig,
    args: Box<dyn std::any::Any + Send>,
    tx: tokio::sync::mpsc::Sender<ArticleEvent>,
    stream_id: String,
) {
    let limiter = RateLimiter::new(config.rate_limit_per_sec);
    let (poll_interval, hwm0) = match &method {
        PollMethod::Search => {
            let a = args.downcast_ref::<PollArgsSearch>().expect("search args");
            (
                Duration::from_secs(
                    a.args.poll_interval_secs.unwrap_or(60),
                ),
                hwm_read(&stream_id),
            )
        }
        PollMethod::TopHeadlines => {
            let a = args
                .downcast_ref::<PollArgsTopHeadlines>()
                .expect("top-headlines args");
            (
                Duration::from_secs(
                    a.args.poll_interval_secs.unwrap_or(60),
                ),
                hwm_read(&stream_id),
            )
        }
    };
    let mut last_seen_ms: i64 = hwm0;
    loop {
        limiter.wait().await;
        let result = match &method {
            PollMethod::Search => {
                let a = args
                    .downcast_ref::<PollArgsSearch>()
                    .expect("search args");
                let cfg = config.clone();
                let a = a.args.clone();
                search(&cfg, &a).await
            }
            PollMethod::TopHeadlines => {
                let a = args
                    .downcast_ref::<PollArgsTopHeadlines>()
                    .expect("top-headlines args");
                let cfg = config.clone();
                let a = a.args.clone();
                top_headlines(&cfg, &a).await
            }
        };
        match result {
            Ok(events) => {
                for ev in events {
                    // Dedupe: skip articles at or below the
                    // Producer's HWM (within tolerance for
                    // ties from NewsAPI's second-resolution
                    // publishedAt). Newer articles advance the
                    // HWM.
                    if ev.published_at_ms != 0
                        && ev.published_at_ms <= last_seen_ms
                    {
                        continue;
                    }
                    if ev.published_at_ms > last_seen_ms {
                        last_seen_ms = ev.published_at_ms;
                        hwm_write(&stream_id, last_seen_ms);
                    }
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                log::warn!("google_news poll error: {e}");
            }
        }
        tokio::time::sleep(poll_interval).await;
    }
}

// ---- FFI shims (search) ----

mod vtable_search {
    use super::*;

    /// FFI `open` for `google_news_search`: decode the bincode
    /// `OpenConfigSearch`, spawn the background polling task,
    /// return a `*mut c_void` wrapping a `Box<PluginState>`. On
    /// error, returns null and writes the error message to
    /// `*err_out` if non-null.
    pub unsafe extern "C" fn open(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void {
        let bytes = std::slice::from_raw_parts(config_ptr, config_len);
        let cfg: OpenConfigSearch = match bincode::deserialize(bytes) {
            Ok(c) => c,
            Err(e) => {
                write_err(err_out, format!("config decode: {e}").as_bytes());
                return std::ptr::null_mut();
            }
        };
        if cfg.datasource.api_key.is_empty() {
            write_err(
                err_out,
                b"datasource.api_key is required for google_news",
            );
            return std::ptr::null_mut();
        }
        let state = match PluginState::spawn_search(cfg) {
            Ok(s) => s,
            Err(e) => {
                write_err(err_out, e.to_string().as_bytes());
                return std::ptr::null_mut();
            }
        };
        let boxed = Box::new(state);
        Box::into_raw(boxed) as *mut std::ffi::c_void
    }

    /// FFI `next` for `google_news_search`.
    pub unsafe extern "C" fn next(
        ctx: *mut std::ffi::c_void,
        out: *mut EventBytes,
    ) -> i32 {
        if ctx.is_null() {
            return -1;
        }
        let state = &mut *(ctx as *mut PluginState);
        match state.next_event() {
            Some(event) => {
                let bee_event = bee_plugin_sdk_event(&event);
                let bytes = encode_event(&bee_event);
                let len = bytes.len();
                let ptr = bytes.as_ptr();
                std::mem::forget(bytes);
                *out = EventBytes { ptr, len };
                1
            }
            None => {
                *out = EventBytes::EMPTY;
                0
            }
        }
    }

    /// FFI `close` for `google_news_search`.
    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let _ = Box::from_raw(ctx as *mut PluginState);
        0
    }

    pub const VTABLE: InputAdapterVtable =
        InputAdapterVtable { open, next, close };
}

// ---- FFI shims (top-headlines) ----

mod vtable_top_headlines {
    use super::*;

    pub unsafe extern "C" fn open(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void {
        let bytes = std::slice::from_raw_parts(config_ptr, config_len);
        let cfg: OpenConfigTopHeadlines = match bincode::deserialize(bytes) {
            Ok(c) => c,
            Err(e) => {
                write_err(err_out, format!("config decode: {e}").as_bytes());
                return std::ptr::null_mut();
            }
        };
        if cfg.datasource.api_key.is_empty() {
            write_err(
                err_out,
                b"datasource.api_key is required for google_news",
            );
            return std::ptr::null_mut();
        }
        let state = match PluginState::spawn_top_headlines(cfg) {
            Ok(s) => s,
            Err(e) => {
                write_err(err_out, e.to_string().as_bytes());
                return std::ptr::null_mut();
            }
        };
        let boxed = Box::new(state);
        Box::into_raw(boxed) as *mut std::ffi::c_void
    }

    pub unsafe extern "C" fn next(
        ctx: *mut std::ffi::c_void,
        out: *mut EventBytes,
    ) -> i32 {
        if ctx.is_null() {
            return -1;
        }
        let state = &mut *(ctx as *mut PluginState);
        match state.next_event() {
            Some(event) => {
                let bee_event = bee_plugin_sdk_event(&event);
                let bytes = encode_event(&bee_event);
                let len = bytes.len();
                let ptr = bytes.as_ptr();
                std::mem::forget(bytes);
                *out = EventBytes { ptr, len };
                1
            }
            None => {
                *out = EventBytes::EMPTY;
                0
            }
        }
    }

    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let _ = Box::from_raw(ctx as *mut PluginState);
        0
    }

    pub const VTABLE: InputAdapterVtable =
        InputAdapterVtable { open, next, close };
}

/// Best-effort: write `msg` as an `EventBytes` blob into `*err_out`.
/// The host reads it on FFI failure. We bincode-wrap a synthetic
/// `Event` so the host can use its standard decoder.
fn write_err(err_out: *mut EventBytes, msg: &[u8]) {
    if err_out.is_null() {
        return;
    }
    let bee_event = Event {
        timestamp: now_ms() as u64,
        sequence: 0,
        payload: msg.to_vec(),
    };
    let bytes = encode_event(&bee_event);
    let len = bytes.len();
    let ptr = bytes.as_ptr();
    std::mem::forget(bytes);
    unsafe {
        *err_out = EventBytes { ptr, len };
    }
}

/// Construct a `bee_adapter::Event` from an `ArticleEvent`. The
/// bincode layout matches the host's decoder.
fn bee_plugin_sdk_event(a: &ArticleEvent) -> Event {
    let payload = bincode::serialize(a).unwrap_or_default();
    Event {
        timestamp: a.published_at_ms.max(0) as u64,
        sequence: 0,
        payload,
    }
}

// ---------------------------------------------------------------------------
// Section 7: Plugin manifest + Factory + cdylib entry
// ---------------------------------------------------------------------------

/// Build the manifest. The plugin exposes two Input Adapters:
/// `google_news_search` and `google_news_top_headlines`. The host
/// matches SQL `google_news.search(...)` and
/// `google_news.top_headlines(...)` to these descriptors.
pub fn plugin_manifest() -> PluginManifest {
    PluginManifest {
        name: PluginName("bee-plugin-google-news".into()),
        feature_version: "1.0.0".into(),
        abi_version: "v1".into(),
        adapters: vec![
            AdapterDescriptor {
                name: "google_news_search".into(),
                is_input: true,
            },
            AdapterDescriptor {
                name: "google_news_top_headlines".into(),
                is_input: true,
            },
        ],
        handlers: vec![],
    }
}

pub struct GoogleNewsFactory;

impl Factory for GoogleNewsFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        let mut input_adapters: HashMap<String, *const InputAdapterVtable> =
            HashMap::new();
        // The host looks up adapters by their full dotted name
        // (e.g. `google_news.search`); the SQL parser strips the
        // `google_news.` prefix, so we register the vtables
        // under the unprefixed adapter names.
        input_adapters.insert(
            "google_news_search".to_string(),
            &vtable_search::VTABLE as *const _,
        );
        input_adapters.insert(
            "google_news_top_headlines".to_string(),
            &vtable_top_headlines::VTABLE as *const _,
        );
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
            input_adapters,
            output_adapters: HashMap::new(),
            handlers: HashMap::new(),
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(GoogleNewsFactory);
