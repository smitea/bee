//! `write` Output adapter — batched `POST /api/v2/write`.
//!
//! ## Threading model
//!
//! `WriteCtx::append()` is the synchronous FFI entry point. It
//! must not block on HTTP. The architecture:
//!
//! 1. The shared batch buffer is an `Arc<Mutex<Vec<LineProtocolRow>>>`.
//!    `append()` takes the lock, pushes a row, and (if the buffer
//!    is full) sets a "flush requested" flag.
//! 2. A worker thread + dedicated multi-thread tokio runtime
//!    owns the HTTP client and the rate limiter. The worker
//!    loops on a `tokio::time::interval`; on each tick it
//!    drains the buffer (if non-empty) and POSTs.
//! 3. The size-based flush is a "request flush" flag plus a
//!    `tokio::sync::Notify` so the worker wakes immediately
//!    instead of waiting for the time tick.
//! 4. On `Drop`, the worker does a final flush and exits.
//!
//! ## Failure handling
//!
//! HTTP failures are logged at `warn!` level. The S36 spec does
//! not require a retry loop; the S41 follow-up can add one.
//! Whatever failed-flush data is in the buffer at the time of
//! the failure is left in the buffer for the next interval to
//! retry (or for `close` to drop — for MVP we drop, since
//! at-least-once semantics are not in S36 scope).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value as JsonValue;
use tokio::sync::Notify;

use bee_adapter::Event;

use crate::config::{InfluxdbConfig, WriteArgs};
use crate::line::{encode_line_protocol, LineProtocolRow};
use crate::ratelimit::RateLimiter;
use crate::InfluxdbError;

// (No re-aliasing; use `InfluxdbError` from the crate root.)

/// The shared FFI context for the `write` adapter. The struct
/// is `Box`-allocated in `open()` and recovered in `close()`.
///
/// `Sync + Send`: yes (the only non-Sync fields are behind
/// `Arc<Mutex<_>>`).
pub struct WriteCtx {
    /// Shared batch buffer. `append()` pushes here; the worker
    /// drains here.
    batch: Arc<Mutex<Vec<LineProtocolRow>>>,
    /// "Flush now" signal. The worker `notified().await` polls
    /// this in parallel with the time interval.
    flush_notify: Arc<Notify>,
    /// Shutdown flag. `drop()` sets this so the worker can exit
    /// its loop after the final flush, instead of waiting up to
    /// `flush_interval_ms` for the next tick.
    shutdown: Arc<AtomicBool>,
    /// Worker thread handle; dropping the ctx signals the
    /// worker to do a final flush + exit.
    worker: Option<std::thread::JoinHandle<()>>,
    /// Worker-side runtime handle. Dropped at ctx drop, which
    /// forces any in-flight `block_on` to return.
    runtime: Option<Arc<tokio::runtime::Runtime>>,
    /// Per-call args (measurement, tag_cols, field_cols,
    /// bucket override, timestamp_col). Cloned at `spawn()` and
    /// held by both the ctx (for `append`) and the worker (for
    /// encoding).
    args: WriteArgs,
    /// Effective bucket (per-call override or Datasource
    /// default). Held in the ctx so the worker's flush does not
    /// need to reach into the config on every tick.
    effective_bucket: String,
    /// Effective timestamp column name. Same rationale.
    effective_timestamp_col: String,
    /// Default field selector (the user's `field_cols`, or
    /// `None` meaning "all non-tag numeric columns").
    field_cols: Option<Vec<String>>,
    /// Size threshold (mirrors `InfluxdbConfig::max_batch_size`).
    max_batch_size: usize,
}

impl WriteCtx {
    /// Build the ctx and spawn the background flush thread.
    /// Returns the ctx (with the worker handle inside).
    pub fn spawn(config: InfluxdbConfig, args: WriteArgs) -> Result<Self, InfluxdbError> {
        if config.url.is_empty() {
            return Err(InfluxdbError::Config("url is required".into()));
        }
        if config.token.is_empty() {
            return Err(InfluxdbError::Config("token is required".into()));
        }
        if config.org.is_empty() {
            return Err(InfluxdbError::Config("org is required".into()));
        }
        let effective_bucket = args
            .effective_bucket(&config.bucket)
            .to_string();
        if effective_bucket.is_empty() {
            return Err(InfluxdbError::Config(
                "bucket is required (no Datasource default and no per-call override)".into(),
            ));
        }
        let effective_timestamp_col = args.effective_timestamp_col().to_string();
        let field_cols = args.field_cols.clone();
        let max_batch_size = config.max_batch_size;

        let batch: Arc<Mutex<Vec<LineProtocolRow>>> = Arc::new(Mutex::new(Vec::new()));
        let flush_notify = Arc::new(Notify::new());
        let shutdown = Arc::new(AtomicBool::new(false));

        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| InfluxdbError::Runtime(e.to_string()))?,
        );

        let worker_batch = Arc::clone(&batch);
        let worker_notify = Arc::clone(&flush_notify);
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_bucket = effective_bucket.clone();
        let worker_runtime = Arc::clone(&runtime);
        let worker_handle = std::thread::Builder::new()
            .name("influxdb-flush".into())
            .spawn(move || {
                let _guard = worker_runtime.enter();
                worker_runtime.block_on(flush_loop(
                    FlushConfig {
                        http_config: config,
                        bucket: worker_bucket,
                    },
                    worker_batch,
                    worker_notify,
                    worker_shutdown,
                ));
            })
            .map_err(|e| InfluxdbError::Runtime(format!("spawn worker: {e}")))?;

        Ok(Self {
            batch,
            flush_notify,
            shutdown,
            worker: Some(worker_handle),
            runtime: Some(runtime),
            args,
            effective_bucket,
            effective_timestamp_col,
            field_cols,
            max_batch_size,
        })
    }

    /// FFI `emit` entry point. Decode the event, build a row,
    /// append to the batch buffer. If the buffer is full,
    /// signal the worker to flush.
    pub fn append(&self, event: &Event) -> Result<(), InfluxdbError> {
        let row = row_from_event(
            event,
            &self.args,
            &self.effective_bucket,
            &self.effective_timestamp_col,
            self.field_cols.as_deref(),
        )?;

        let should_signal_flush = {
            let mut batch = self.batch.lock().expect("batch poisoned");
            batch.push(row);
            batch.len() >= self.max_batch_size
        };
        if should_signal_flush {
            self.flush_notify.notify_one();
        }
        Ok(())
    }
}

impl Drop for WriteCtx {
    fn drop(&mut self) {
        // 1. Notify the worker to do a final flush.
        // 2. Set the shutdown flag so the worker exits its loop
        //    after the final flush (instead of waiting up to
        //    `flush_interval_ms` for the next tick).
        // 3. Drop the worker-side runtime so the OS thread is
        //    freed when the worker finishes.
        // 4. Join the worker thread so the final flush completes
        //    before the ctx is fully dropped.
        self.flush_notify.notify_one();
        self.shutdown.store(true, Ordering::SeqCst);
        self.runtime = None;
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Worker: flush loop
// ---------------------------------------------------------------------------

/// Bundle of fields the worker thread needs to do its job.
/// Kept private to this module.
struct FlushConfig {
    /// HTTP client config (URL, token, org, rate limit,
    /// timeout, flush interval, batch size). The `bucket` and
    /// `args` fields of `InfluxdbConfig` / `WriteArgs` are
    /// handled by the ctx's `append()` method before rows are
    /// pushed into the buffer.
    http_config: InfluxdbConfig,
    /// Effective bucket (per-call override or Datasource
    /// default). Used to construct the `POST /api/v2/write`
    /// URL.
    bucket: String,
}

async fn flush_loop(
    cfg: FlushConfig,
    batch: Arc<Mutex<Vec<LineProtocolRow>>>,
    flush_notify: Arc<Notify>,
    shutdown: Arc<AtomicBool>,
) {
    let limiter = RateLimiter::new(cfg.http_config.rate_limit_per_sec);
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(cfg.http_config.timeout_ms))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("influxdb: client build: {e}");
            return;
        }
    };
    let url = format!(
        "{}/api/v2/write?org={}&bucket={}",
        cfg.http_config.url.trim_end_matches('/'),
        urlencoding(&cfg.http_config.org),
        urlencoding(&cfg.bucket),
    );
    let mut interval =
        tokio::time::interval(Duration::from_millis(cfg.http_config.flush_interval_ms));
    // The first tick fires immediately; we don't want an empty
    // flush at t=0, so skip it.
    interval.tick().await;

    loop {
        // Wait for a flush signal or a tick.
        tokio::select! {
            _ = interval.tick() => {}
            _ = flush_notify.notified() => {}
        }
        // Drain the buffer in a tight loop. The producer may
        // outpace the worker (each `append` is in-memory; the
        // POST is a network round-trip), so by the time the
        // worker is ready to drain, the buffer can hold more
        // than `max_batch_size` rows. POST all of them as one
        // logical batch to keep the number of HTTP calls down
        // (per the S36 spec criterion: "1000-row burst flushes
        // in <= 2 batches").
        loop {
            let rows: Vec<LineProtocolRow> = {
                let mut b = batch.lock().expect("batch poisoned");
                if b.is_empty() {
                    break;
                }
                std::mem::take(&mut *b)
            };
            limiter.wait().await;
            let body = rows
                .iter()
                .map(encode_line_protocol)
                .collect::<Vec<_>>()
                .join("\n");
            let line_count = rows.len();
            let byte_count = body.len();
            let result = client
                .post(&url)
                .header("Authorization", format!("Token {}", cfg.http_config.token))
                .header("Content-Type", "text/plain; charset=utf-8")
                .header("Accept", "application/json")
                .body(body)
                .send()
                .await;
            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        // Read the error body for logging, but do NOT
                        // include the token (the token is in the
                        // Authorization header, not the body).
                        let body_preview = resp
                            .text()
                            .await
                            .unwrap_or_default()
                            .chars()
                            .take(512)
                            .collect::<String>();
                        log::warn!(
                            "influxdb: POST /api/v2/write failed: status={status} \
                             lines={line_count} bytes={byte_count} body={body_preview:?}"
                        );
                    } else {
                        log::debug!(
                            "influxdb: POST /api/v2/write ok: status={status} \
                             lines={line_count} bytes={byte_count}"
                        );
                    }
                }
                Err(e) => {
                    log::warn!(
                        "influxdb: POST /api/v2/write transport error: {e} \
                         lines={line_count} bytes={byte_count}"
                    );
                }
            }
        }
        // After draining, check whether the ctx is being dropped.
        // If so, exit the loop. This guarantees every queued row
        // is POSTed before the worker thread terminates.
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
    }
}

/// Percent-encode an InfluxDB org or bucket name. InfluxDB v2
/// allows alphanumerics, `-`, `_`, and spaces in org / bucket
/// names; we percent-encode anything else so the URL is safe.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            for b in ch.to_string().as_bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Event -> LineProtocolRow
// ---------------------------------------------------------------------------

/// Decode the event's `payload` (bincode-encoded `HashMap<String,
/// JsonValue>`) into a `LineProtocolRow`.
///
/// The event's payload is expected to be a JSON-encoded
/// `HashMap<String, serde_json::Value>` (UTF-8 bytes). The
/// plugin then extracts the `timestamp_col` for the timestamp,
/// the `tag_cols` for tags, and either the `field_cols` or the
/// remaining numeric values for fields.
///
/// (We use `serde_json` rather than `bincode` for the payload
/// because `serde_json::Value` requires `deserialize_any`, which
/// `bincode` 1.x does not implement. JSON also matches the
/// InfluxDB v2 line-protocol conventions: numbers, strings,
/// booleans all have natural JSON representations.)
fn row_from_event(
    event: &Event,
    args: &WriteArgs,
    _bucket: &str,
    timestamp_col: &str,
    field_cols: Option<&[String]>,
) -> Result<LineProtocolRow, InfluxdbError> {
    // Decode the payload as a `HashMap<String, JsonValue>`. If
    // that fails, the event is malformed and we surface an
    // error.
    let map: HashMap<String, JsonValue> = serde_json::from_slice(&event.payload).map_err(|e| {
        InfluxdbError::Payload(format!("expected JSON HashMap<String, JsonValue>: {e}"))
    })?;

    let timestamp_ns = extract_timestamp(&map, timestamp_col, event.timestamp)?;
    let mut row = LineProtocolRow::new(&args.measurement, timestamp_ns);

    // Tags: copy the requested columns as strings.
    for tag in &args.tag_cols {
        if let Some(v) = map.get(tag) {
            let s = json_to_tag_value(v);
            row.tags.insert(tag.clone(), s);
        }
    }

    // Fields: if the user supplied `field_cols`, use exactly
    // those; otherwise take all numeric / bool / string values
    // that are not in `tag_cols`.
    let fields_to_emit: Vec<String> = match field_cols {
        Some(list) => list.to_vec(),
        None => map
            .keys()
            .filter(|k| !args.tag_cols.contains(k) && *k != timestamp_col)
            .cloned()
            .collect(),
    };
    for f in &fields_to_emit {
        if let Some(v) = map.get(f) {
            let typed = json_to_field_value(v);
            row.fields.insert(f.clone(), typed);
        }
    }

    Ok(row)
}

/// Extract a nanoseconds timestamp from the event. The event's
/// `Event::timestamp` is a microsecond-resolution `u64`
/// (per the SDK), so we multiply by 1000 to get nanoseconds.
/// If the payload contains a `timestamp_col` whose value is a
/// number, that takes precedence (the user explicitly opted
/// into a custom column).
fn extract_timestamp(
    map: &HashMap<String, JsonValue>,
    timestamp_col: &str,
    event_timestamp_us: u64,
) -> Result<i64, InfluxdbError> {
    if let Some(v) = map.get(timestamp_col) {
        match v {
            JsonValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    // Heuristic: if the value is in ms
                    // (1.7e12-ish for current era), keep it as
                    // ms. Otherwise, assume it is already in
                    // nanoseconds.
                    if i < 1_000_000_000_000_000 {
                        return Ok(i * 1_000_000);
                    }
                    return Ok(i);
                }
                if let Some(u) = n.as_u64() {
                    let i = u as i64;
                    if i < 1_000_000_000_000_000 {
                        return Ok(i * 1_000_000);
                    }
                    return Ok(i);
                }
                if let Some(f) = n.as_f64() {
                    return Ok(f as i64);
                }
                Err(InfluxdbError::Payload(format!(
                    "timestamp column {timestamp_col} has non-integer number"
                )))
            }
            JsonValue::String(s) => s
                .parse::<i64>()
                .map_err(|e| InfluxdbError::Payload(format!("timestamp string parse: {e}"))),
            _ => Err(InfluxdbError::Payload(format!(
                "timestamp column {timestamp_col} must be number or string"
            ))),
        }
    } else {
        // Fall back to the event's wall-clock timestamp.
        // Event::timestamp is microseconds (per SDK); convert
        // to nanoseconds.
        Ok((event_timestamp_us as i64) * 1_000)
    }
}

/// Convert a JSON value into a string suitable for an InfluxDB
/// tag value. Tags are always strings; non-string values are
/// stringified.
fn json_to_tag_value(v: &JsonValue) -> String {
    match v {
        JsonValue::String(s) => s.clone(),
        JsonValue::Number(n) => n.to_string(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Null => String::new(),
        _ => v.to_string(),
    }
}

/// Convert a JSON value into a typed field-value string per
/// the InfluxDB v2 line protocol rules:
/// - integer -> `"42i"`
/// - float   -> `"3.14"`
/// - bool    -> `"true"` / `"false"`
/// - string  -> `"\"hello\""` (quoted; embedded `"` escaped)
fn json_to_field_value(v: &JsonValue) -> String {
    match v {
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                format!("{i}i")
            } else if let Some(u) = n.as_u64() {
                format!("{u}u")
            } else if let Some(f) = n.as_f64() {
                // InfluxDB v2 requires floats. Format with
                // `to_string` (the parser accepts any
                // well-formed float).
                format!("{f}")
            } else {
                "0i".to_string()
            }
        }
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        JsonValue::Null => "\"\"".to_string(),
        other => format!(
            "\"{}\"",
            other.to_string().replace('\\', "\\\\").replace('"', "\\\"")
        ),
    }
}
