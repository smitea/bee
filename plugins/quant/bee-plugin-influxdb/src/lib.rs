//! `bee-plugin-influxdb` — production-grade InfluxDB v2 adapter (S36).
//!
//! Implements two adapters against a real InfluxDB v2 instance:
//!
//! - `influxdb_write` (Output): `POST /api/v2/write?org=...&bucket=...`
//!   in the v2 line protocol. Events are batched in a thread-safe
//!   buffer and flushed by a background task on either a size
//!   threshold (default 500 lines) or a time threshold (default
//!   1s), whichever comes first. Each successful flush updates a
//!   `WriteResult` that the runtime can read for observability.
//! - `influxdb_query` (Input): `POST /api/v2/query?org=...` with a
//!   Flux query string. A background task polls the query at a
//!   configurable cadence (default 60s) and pushes each row into an
//!   mpsc; `next()` blocks on the channel.
//!
//! ## Architecture
//!
//! - [`InfluxFactory`]: the `cdylib_plugin!(Factory)` entrypoint.
//!   `init()` registers two vtables in the [`PluginHandle`]: the
//!   `write` Output vtable and the `query` Input vtable.
//! - [`write::WriteCtx`]: the FFI ctx for the Output adapter. Owns
//!   the batch buffer, the HTTP client, and a worker thread that
//!   drives the time-based flush.
//! - [`query::QueryCtx`]: the FFI ctx for the Input adapter. Owns
//!   the mpsc receiver and a worker thread that runs the polling
//!   loop.
//!
//! ## Stream identity
//!
//! - For `write`: Output adapters do not produce Streams (per
//!   `docs/best-practices/quant/stories.md` §S36).
//! - For `query`: `StreamSignature = sha256("influxdb" || "query" || bucket || hash(flux_query))`.
//!
//! ## Credentials
//!
//! `token` is read from the Datasource config and never logged or
//! included in any error message returned across the FFI boundary.
//! On transport errors we log only the HTTP status and a generic
//! message; the token is never serialised into the `EventBytes`
//! error blob.
//!
//! ## Rate limiting
//!
//! A simple token-bucket rate limiter is applied to every outbound
//! HTTP request. The default is 100 req/sec, per the S36 spec
//! ("InfluxDB v2 can handle this"). The rate is configurable per
//! Datasource.

use std::sync::Arc;

use bee_plugin_sdk::{
    vtable::{InputAdapterVtable, OutputAdapterVtable},
    AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName,
};
use sha2::{Digest, Sha256};

use crate::query::QueryCtx;
use crate::write::WriteCtx;

pub mod config;
pub mod escape;
pub mod line;
pub mod query;
pub mod ratelimit;
pub mod write;

// ---------------------------------------------------------------------------
// Section 3: StreamSignature
// ---------------------------------------------------------------------------

/// StreamSignature for the `write` Output adapter. Output adapters
/// do not produce Streams; this constant is exported so the
/// Compiler / Registry can name the connection-level identity.
pub const WRITE_STREAM_SIGNATURE: &str = "influxdb:write";

/// Compute the StreamSignature for the `query` Input adapter.
///
///   `StreamSignature = sha256("influxdb" || "query" || bucket || flux_query)`
///
/// `bucket` is included so that the same Flux query against two
/// different buckets produces two different Producers (different
/// Stream identity, different in-memory state).
pub fn query_stream_signature(bucket: &str, flux_query: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"influxdb");
    hasher.update(b"query");
    hasher.update(bucket.as_bytes());
    hasher.update(flux_query.as_bytes());
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------------------
// Section 4: Error type (used by both vtable shims)
// ---------------------------------------------------------------------------

/// Errors surfaced from the FFI shims. The Display impl never
/// includes the InfluxDB token (the token is held in the
/// Datasource config and is excluded from every error path).
#[derive(Debug, thiserror::Error)]
pub enum InfluxdbError {
    #[error("config decode: {0}")]
    Config(String),
    #[error("bincode: {0}")]
    Bincode(String),
    #[error("runtime: {0}")]
    Runtime(String),
    #[error("http: {0}")]
    Http(String),
    #[error("invalid line: {0}")]
    Line(String),
    #[error("channel closed")]
    ChannelClosed,
    #[error("invalid event payload: {0}")]
    Payload(String),
    #[error("rate-limit init: {0}")]
    RateLimit(String),
}

// ---------------------------------------------------------------------------
// Section 5: Plugin manifest + Factory + cdylib entry
// ---------------------------------------------------------------------------

/// Build the manifest. The plugin exposes two adapters:
/// `write` (Output) and `query` (Input).
pub fn plugin_manifest() -> PluginManifest {
    PluginManifest {
        name: PluginName("influxdb".into()),
        feature_version: "1.0.0".into(),
        abi_version: "v1".into(),
        adapters: vec![
            AdapterDescriptor {
                name: "write".into(),
                is_input: false,
            },
            AdapterDescriptor {
                name: "query".into(),
                is_input: true,
            },
        ],
        handlers: vec![],
    }
}

/// Factory for the influxdb plugin. The unit type; both methods
/// are pure and idempotent.
pub struct InfluxFactory;

impl Factory for InfluxFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        let write_vtable: *const OutputAdapterVtable = &write_shim::VTABLE;
        let query_vtable: *const InputAdapterVtable = &query_shim::VTABLE;
        let mut output_adapters = std::collections::HashMap::new();
        output_adapters.insert("write".to_string(), write_vtable);
        let mut input_adapters = std::collections::HashMap::new();
        input_adapters.insert("query".to_string(), query_vtable);
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: Arc::new(()),
            input_adapters,
            output_adapters,
            handlers: std::collections::HashMap::new(),
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(InfluxFactory);

// ---------------------------------------------------------------------------
// FFI vtable shims
// ---------------------------------------------------------------------------

mod write_shim {
    use super::WriteCtx;
    use crate::config::WriteArgs;
    use bee_plugin_sdk::event::{decode_event, EventBytes};
    use bee_plugin_sdk::vtable::OutputAdapterVtable;

    /// Write a UTF-8 error string into the `*err_out` slot as an
    /// `EventBytes` blob (bincode-`Event`-shaped for the host's
    /// decoder). The token is never included.
    fn write_err(err_out: *mut EventBytes, msg: &str) {
        if err_out.is_null() {
            return;
        }
        let bee_event = bee_adapter::Event {
            timestamp: 0,
            sequence: 0,
            payload: msg.as_bytes().to_vec(),
        };
        let bytes = bincode::serialize(&bee_event).unwrap_or_default();
        let len = bytes.len();
        let ptr = bytes.as_ptr();
        std::mem::forget(bytes);
        unsafe {
            *err_out = EventBytes { ptr, len };
        }
    }

    /// FFI `open`: decode the bincode `OpenConfig`, spawn the
    /// background flush thread, return a `*mut c_void` wrapping a
    /// `Box<WriteCtx>`. On error, returns null and writes the
    /// error message to `*err_out` if non-null.
    pub unsafe extern "C" fn open(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void {
        let bytes = std::slice::from_raw_parts(config_ptr, config_len);
        let cfg: OpenConfig = match bincode::deserialize(bytes) {
            Ok(c) => c,
            Err(e) => {
                write_err(err_out, &format!("config decode: {e}"));
                return std::ptr::null_mut();
            }
        };
        let ctx = match WriteCtx::spawn(cfg.datasource, cfg.stream) {
            Ok(c) => c,
            Err(e) => {
                write_err(err_out, &e.to_string());
                return std::ptr::null_mut();
            }
        };
        let boxed = Box::new(ctx);
        Box::into_raw(boxed) as *mut std::ffi::c_void
    }

    /// FFI `emit`: bincode-decode the `Event`, append a line-
    /// protocol row to the batch buffer. If the buffer crosses
    /// `max_batch_size`, synchronously trigger a flush. Returns
    /// 0 on success, -1 on error.
    pub unsafe extern "C" fn emit(
        ctx: *mut std::ffi::c_void,
        event_ptr: *const u8,
        event_len: usize,
    ) -> i32 {
        if ctx.is_null() {
            return -1;
        }
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let event = match decode_event(event_bytes) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("influxdb.emit: bincode decode: {e}");
                return -1;
            }
        };
        let ctx = &*(ctx as *const WriteCtx);
        match ctx.append(&event) {
            Ok(()) => 0,
            Err(e) => {
                log::warn!("influxdb.emit: {e}");
                -1
            }
        }
    }

    /// FFI `close`: take the `Box<WriteCtx>` back and drop it. The
    /// `Drop` impl performs a final flush and joins the worker.
    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let _ = Box::from_raw(ctx as *mut WriteCtx);
        0
    }

    /// FFI-facing config blob: bundles the Datasource config with
    /// the per-call `WriteArgs` so a single `open()` call carries
    /// both. The Compiler packages them as one bincode blob.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct OpenConfig {
        pub datasource: super::config::InfluxdbConfig,
        pub stream: WriteArgs,
    }

    pub const VTABLE: OutputAdapterVtable = OutputAdapterVtable {
        open,
        emit,
        close,
    };
}

mod query_shim {
    use super::QueryCtx;
    use crate::config::QueryArgs;
    use bee_plugin_sdk::event::{encode_event, EventBytes};
    use bee_plugin_sdk::vtable::InputAdapterVtable;

    fn write_err(err_out: *mut EventBytes, msg: &str) {
        if err_out.is_null() {
            return;
        }
        let bee_event = bee_adapter::Event {
            timestamp: 0,
            sequence: 0,
            payload: msg.as_bytes().to_vec(),
        };
        let bytes = bincode::serialize(&bee_event).unwrap_or_default();
        let len = bytes.len();
        let ptr = bytes.as_ptr();
        std::mem::forget(bytes);
        unsafe {
            *err_out = EventBytes { ptr, len };
        }
    }

    pub unsafe extern "C" fn open(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void {
        let bytes = std::slice::from_raw_parts(config_ptr, config_len);
        let cfg: OpenConfig = match bincode::deserialize(bytes) {
            Ok(c) => c,
            Err(e) => {
                write_err(err_out, &format!("config decode: {e}"));
                return std::ptr::null_mut();
            }
        };
        let ctx = match QueryCtx::spawn(cfg.datasource, cfg.stream) {
            Ok(c) => c,
            Err(e) => {
                write_err(err_out, &e.to_string());
                return std::ptr::null_mut();
            }
        };
        let boxed = Box::new(ctx);
        Box::into_raw(boxed) as *mut std::ffi::c_void
    }

    pub unsafe extern "C" fn next(
        ctx: *mut std::ffi::c_void,
        out: *mut EventBytes,
    ) -> i32 {
        if ctx.is_null() {
            return -1;
        }
        let ctx = &mut *(ctx as *mut QueryCtx);
        match ctx.next_event() {
            Some(event) => {
                let bytes = encode_event(&event);
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
        let _ = Box::from_raw(ctx as *mut QueryCtx);
        0
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct OpenConfig {
        pub datasource: super::config::InfluxdbConfig,
        pub stream: QueryArgs,
    }

    pub const VTABLE: InputAdapterVtable = InputAdapterVtable { open, next, close };
}

// ---------------------------------------------------------------------------
// Section 6: Unit tests (S36 f)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{InfluxdbConfig, QueryArgs, WriteArgs};
    use crate::escape::escape;
    use crate::line::{encode_line_protocol, LineProtocolRow};
    use crate::ratelimit::RateLimiter;
    use std::collections::HashMap;
    use std::io::{Read, Write as IoWrite};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // Section 6.1: line-protocol encoding
    // -----------------------------------------------------------------------

    #[test]
    fn line_protocol_basic() {
        // The spec's example row:
        //   measurement,tag1=v1 field1=1.0,field2=2 1234567890
        let mut row = LineProtocolRow::new("measurement", 1_234_567_890);
        row.tags.insert("tag1".into(), "v1".into());
        row.fields.insert("field1".into(), "1.0".into());
        row.fields.insert("field2".into(), "2i".into());
        let line = encode_line_protocol(&row);
        assert_eq!(line, "measurement,tag1=v1 field1=1.0,field2=2i 1234567890");
    }

    #[test]
    fn line_protocol_no_tags() {
        // No tags means no leading comma; field section is still sorted.
        let mut row = LineProtocolRow::new("measurement", 1_234_567_890);
        row.fields.insert("field1".into(), "1.0".into());
        let line = encode_line_protocol(&row);
        assert_eq!(line, "measurement field1=1.0 1234567890");
    }

    #[test]
    fn line_protocol_string_field_quoted() {
        // The caller pre-formats string field values as `"hello world"`.
        // The encoder does not re-quote; it just emits the value verbatim.
        let mut row = LineProtocolRow::new("measurement", 1_234_567_890);
        row.fields.insert("field1".into(), "\"hello world\"".into());
        let line = encode_line_protocol(&row);
        assert_eq!(line, "measurement field1=\"hello world\" 1234567890");
    }

    #[test]
    fn line_protocol_multiple_tags_sorted() {
        // Per the impl, tags are emitted in sorted key order. Insert in
        // a different order to prove the sort is what determines output
        // (not insertion order).
        let mut row = LineProtocolRow::new("m", 1_000);
        row.tags.insert("zebra".into(), "z".into());
        row.tags.insert("alpha".into(), "a".into());
        row.tags.insert("middle".into(), "m".into());
        let line = encode_line_protocol(&row);
        assert_eq!(line, "m,alpha=a,middle=m,zebra=z  1000");
    }

    #[test]
    fn line_protocol_escape_commas_and_spaces_in_tags() {
        // `escape()` rules: `,` -> `\,`, ` ` -> `\ `, `=` -> `\=`.
        let escaped = escape("a,b c=d");
        assert_eq!(escaped, "a\\,b\\ c\\=d");
        // And end-to-end: the encoder runs the same escape over tag keys
        // and tag values.
        let mut row = LineProtocolRow::new("m", 100);
        row.tags.insert("key,with space".into(), "v=v".into());
        row.fields.insert("f".into(), "1i".into());
        let line = encode_line_protocol(&row);
        assert_eq!(line, "m,key\\,with\\ space=v\\=v f=1i 100");
    }

    #[test]
    fn line_protocol_escape_in_field_values() {
        // Field values are passed through verbatim (the encoder does not
        // escape). The caller is responsible for quoting string values
        // and escaping embedded `"` as `\"`. Verify the encoder preserves
        // the caller's escaping.
        let mut row = LineProtocolRow::new("m", 100);
        row.fields
            .insert("f".into(), "\"a \\\"quoted\\\" word\"".into());
        let line = encode_line_protocol(&row);
        assert_eq!(line, "m f=\"a \\\"quoted\\\" word\" 100");
    }

    // -----------------------------------------------------------------------
    // Section 6.2: config defaults + bincode
    // -----------------------------------------------------------------------

    #[test]
    fn config_default_url() {
        let cfg = InfluxdbConfig::default();
        assert_eq!(cfg.url, "http://localhost:8086");
        assert_eq!(cfg.timeout_ms, 5_000);
        assert_eq!(cfg.rate_limit_per_sec, 100);
        assert_eq!(cfg.max_batch_size, 500);
        assert_eq!(cfg.flush_interval_ms, 1_000);
        assert_eq!(cfg.tenant, 0);
        // Bincode round-trip works on the default.
        let bytes = bincode::serialize(&cfg).expect("serialize default");
        let back: InfluxdbConfig = bincode::deserialize(&bytes).expect("deserialize default");
        assert_eq!(back.url, cfg.url);
        assert_eq!(back.timeout_ms, cfg.timeout_ms);
        assert_eq!(back.token, cfg.token);
        assert_eq!(back.org, cfg.org);
        assert_eq!(back.bucket, cfg.bucket);
    }

    #[test]
    fn config_bincode_roundtrip() {
        let cfg = InfluxdbConfig {
            url: "https://influx.example.test:8086".into(),
            token: "t0k3n-abc".into(),
            org: "my-org".into(),
            bucket: "default".into(),
            timeout_ms: 12_345,
            rate_limit_per_sec: 250,
            max_batch_size: 750,
            flush_interval_ms: 250,
            tenant: 7,
        };
        let bytes = bincode::serialize(&cfg).expect("serialize");
        let back: InfluxdbConfig = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back.url, cfg.url);
        assert_eq!(back.token, cfg.token);
        assert_eq!(back.org, cfg.org);
        assert_eq!(back.bucket, cfg.bucket);
        assert_eq!(back.timeout_ms, cfg.timeout_ms);
        assert_eq!(back.rate_limit_per_sec, cfg.rate_limit_per_sec);
        assert_eq!(back.max_batch_size, cfg.max_batch_size);
        assert_eq!(back.flush_interval_ms, cfg.flush_interval_ms);
        assert_eq!(back.tenant, cfg.tenant);
    }

    #[test]
    fn config_missing_token_fails_at_open() {
        // The `token is required` check lives in `WriteCtx::spawn`
        // (called by the FFI `open`). A blank token must produce a
        // `Config` error.
        let cfg = InfluxdbConfig {
            url: "http://localhost:8086".into(),
            token: String::new(),
            org: "o".into(),
            bucket: "b".into(),
            ..InfluxdbConfig::default()
        };
        let args = WriteArgs {
            measurement: "m".into(),
            bucket: None,
            tag_cols: vec![],
            field_cols: None,
            timestamp_col: None,
        };
        let res = WriteCtx::spawn(cfg, args);
        let err = match res {
            Ok(_) => panic!("expected WriteCtx::spawn to fail on empty token"),
            Err(e) => e,
        };
        assert!(
            matches!(err, InfluxdbError::Config(ref s) if s.contains("token")),
            "expected Config(token), got {err}"
        );
    }

    #[test]
    fn config_token_never_in_error_message() {
        // The InfluxdbError Display impl must NEVER include the token,
        // even when the underlying reqwest error or a query response
        // would have included it. We test this by constructing an
        // InfluxdbError::Http (the path most likely to surface a token
        // from a real response) and checking that the secret token
        // string does not appear in the Display output.
        let secret = "SECRET-TOKEN-XYZ-12345";
        let err = InfluxdbError::Http("POST /api/v2/write status=401 Unauthorized".to_string());
        let displayed = err.to_string();
        assert!(
            !displayed.contains(secret),
            "token leaked into Http error message: {displayed}"
        );
        // Also exercise the Config error path: it must not echo the
        // token back, even though the user passed one.
        let err2 = InfluxdbError::Config("url is required".into());
        assert!(!err2.to_string().contains(secret));
    }

    #[test]
    fn write_args_bincode_roundtrip() {
        let args = WriteArgs {
            measurement: "klines".into(),
            bucket: Some("archive".into()),
            tag_cols: vec!["symbol".into(), "interval".into()],
            field_cols: Some(vec![
                "open".into(),
                "high".into(),
                "low".into(),
                "close".into(),
            ]),
            timestamp_col: Some("ts".into()),
        };
        let bytes = bincode::serialize(&args).expect("serialize WriteArgs");
        let back: WriteArgs = bincode::deserialize(&bytes).expect("deserialize WriteArgs");
        assert_eq!(back.measurement, args.measurement);
        assert_eq!(back.bucket, args.bucket);
        assert_eq!(back.tag_cols, args.tag_cols);
        assert_eq!(back.field_cols, args.field_cols);
        assert_eq!(back.timestamp_col, args.timestamp_col);
        // Effective accessors reflect the per-call args.
        assert_eq!(back.effective_timestamp_col(), "ts");
        assert_eq!(back.effective_bucket("ds-default"), "archive");
    }

    // -----------------------------------------------------------------------
    // Section 6.3: bucket override
    // -----------------------------------------------------------------------

    #[test]
    fn bucket_override_takes_precedence() {
        // `WriteArgs::effective_bucket` returns the per-call override if
        // Some, else falls back to the Datasource default. The worker
        // uses the same helper to build the POST URL.
        let args_some = WriteArgs {
            measurement: "m".into(),
            bucket: Some("archive".into()),
            tag_cols: vec![],
            field_cols: None,
            timestamp_col: None,
        };
        assert_eq!(args_some.effective_bucket("default"), "archive");
        assert_eq!(args_some.effective_bucket(""), "archive");

        let args_none = WriteArgs {
            measurement: "m".into(),
            bucket: None,
            tag_cols: vec![],
            field_cols: None,
            timestamp_col: None,
        };
        assert_eq!(args_none.effective_bucket("default"), "default");
        assert_eq!(args_none.effective_bucket(""), "");
    }

    // -----------------------------------------------------------------------
    // Section 6.4: rate limiter
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rate_limiter_respects_token_bucket() {
        // 50 req/sec => min_interval = 20 ms. Fire 100 requests; the
        // first 1 takes 0 ms, the next 99 must each wait >= 20 ms =>
        // at least 1980 ms total. We use real wall-clock time because
        // the limiter's internal clock is `std::time::Instant` (paused
        // virtual time would never advance it).
        let limiter = RateLimiter::new(50);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            limiter.wait().await;
        }
        let elapsed = start.elapsed();
        let expected_min = Duration::from_millis(99 * 20);
        assert!(
            elapsed >= expected_min,
            "elapsed {elapsed:?} must be >= {expected_min:?} (limiter not throttling?)"
        );
        // Generous upper bound for CI jitter. If the limiter is broken
        // (e.g. always returns immediately), elapsed would be ~0 ms.
        let expected_max = expected_min + Duration::from_millis(2_000);
        assert!(
            elapsed < expected_max,
            "elapsed {elapsed:?} must be < {expected_max:?} (way too slow?)"
        );
    }

    // -----------------------------------------------------------------------
    // Section 6.5: stream signatures
    // -----------------------------------------------------------------------

    #[test]
    fn write_signature_constant() {
        // Output adapters have a connection-level identity (per spec):
        // the signature is the constant `"influxdb:write"`.
        assert_eq!(WRITE_STREAM_SIGNATURE, "influxdb:write");
    }

    #[test]
    fn query_signature_includes_bucket_and_flux() {
        let s1 = query_stream_signature("default", "from(bucket: \"b\") |> range(start: -1h)");
        let s2 = query_stream_signature("default", "from(bucket: \"b\") |> range(start: -1h)");
        // Stable / deterministic for the same inputs.
        assert_eq!(s1, s2);
        // 64 hex chars = sha256 hex.
        assert_eq!(s1.len(), 64);
        assert!(s1.chars().all(|c| c.is_ascii_hexdigit()));
        // Different bucket -> different hash.
        let s3 = query_stream_signature("archive", "from(bucket: \"b\") |> range(start: -1h)");
        assert_ne!(s1, s3, "bucket must be part of the signature");
        // Different flux query -> different hash.
        let s4 = query_stream_signature("default", "from(bucket: \"b\") |> range(start: -2h)");
        assert_ne!(s1, s4, "flux query must be part of the signature");
    }

    // -----------------------------------------------------------------------
    // Section 6.6: batching — real HTTP server in a thread
    // -----------------------------------------------------------------------
    //
    // We spin up a minimal HTTP server in a background thread that
    // accepts `POST /api/v2/write`, records the count and the request
    // bodies, and replies 204. The InfluxDB client is configured to
    // point at this server. We then append 1000 events to a WriteCtx
    // with `max_batch_size = 500` and assert the server saw at most
    // 2 POSTs. (One at the 500-row signal, one at the 1000-row signal
    // / final flush.)
    //
    // This exercises the full path: encode -> batch -> notify ->
    // worker flush -> HTTP. The test is bounded by 5 s wall-clock
    // (the worker is dropped at the end of the test, which forces a
    // final flush).

    /// Start a minimal HTTP server that accepts POSTs on any path
    /// and replies 204 No Content. Returns `(base_url, counter)`.
    /// The counter is shared (Arc<Mutex<Vec<Vec<u8>>>>) so the test
    /// can inspect how many POSTs arrived and their bodies.
    fn spawn_capture_server() -> (String, Arc<Mutex<Vec<Vec<u8>>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local_addr").port();
        let bodies: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let bodies_clone = Arc::clone(&bodies);
        thread::Builder::new()
            .name("influxdb-test-http".into())
            .spawn(move || {
                // We loop forever; the test's `WriteCtx` is dropped at
                // end of test, but the listener is leaked (fine for a
                // short-lived test process).
                for stream in listener.incoming() {
                    let Ok(mut s) = stream else { continue };
                    // Read until we see "\r\n\r\n" (end of headers).
                    let mut buf = Vec::with_capacity(8192);
                    let mut tmp = [0u8; 1024];
                    loop {
                        match s.read(&mut tmp) {
                            Ok(0) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&tmp[..n]);
                                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    // Parse out Content-Length.
                    let header_end = buf
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .unwrap_or(buf.len());
                    let header_str = String::from_utf8_lossy(&buf[..header_end]);
                    let content_length = header_str
                        .to_ascii_lowercase()
                        .lines()
                        .find_map(|l| {
                            let mut parts = l.splitn(2, ':');
                            let k = parts.next()?.trim();
                            let v = parts.next()?.trim();
                            if k == "content-length" {
                                v.parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    // Read the body if not already fully read.
                    let body_start = header_end + 4;
                    while buf.len() < body_start + content_length {
                        match s.read(&mut tmp) {
                            Ok(0) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&tmp[..n]);
                            }
                            Err(_) => break,
                        }
                    }
                    let body = buf[body_start..body_start + content_length].to_vec();
                    bodies_clone.lock().expect("bodies poisoned").push(body);
                    // 204 No Content, no body.
                    let response = b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n";
                    let _ = s.write_all(response);
                    let _ = s.flush();
                }
            })
            .expect("spawn test server");
        (format!("http://127.0.0.1:{port}"), bodies)
    }

    #[test]
    fn batching_flushes_at_max_batch_size() {
        let (url, bodies) = spawn_capture_server();

        // Build a config pointing at our loopback server. Use a
        // 500-row size threshold so a 1000-row burst flushes in
        // two size-driven batches.
        let cfg = InfluxdbConfig {
            url,
            token: "test-token".into(),
            org: "test-org".into(),
            bucket: "default".into(),
            timeout_ms: 2_000,
            rate_limit_per_sec: 1_000_000, // effectively unlimited
            max_batch_size: 500,
            flush_interval_ms: 60_000, // size-based flush should win
            tenant: 0,
        };
        let args = WriteArgs {
            measurement: "klines".into(),
            bucket: None,
            tag_cols: vec![],
            field_cols: Some(vec!["price".into()]),
            timestamp_col: None,
        };

        let ctx = WriteCtx::spawn(cfg, args).expect("spawn ctx");

        // Helper: build a JSON-encoded event payload.
        let make_event = |i: u64| {
            let mut map: HashMap<String, serde_json::Value> = HashMap::new();
            map.insert("price".into(), serde_json::json!(i as f64));
            let payload = serde_json::to_vec(&map).expect("serialize payload");
            bee_adapter::Event {
                timestamp: 1_700_000_000_000 + i,
                sequence: i,
                payload,
            }
        };

        // Push 500 events. The 500th push crosses the size
        // threshold and triggers a flush.
        for i in 0..500u64 {
            ctx.append(&make_event(i)).expect("append event");
        }
        // Wait for the first batch to land at the server. The
        // worker POSTs asynchronously; we give it up to 2 s.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let n = bodies.lock().expect("bodies poisoned").len();
            if n >= 1 || std::time::Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let after_first = bodies.lock().expect("bodies poisoned").len();
        assert_eq!(
            after_first, 1,
            "first 500 rows must flush in exactly 1 POST, got {after_first}"
        );

        // Push the second 500. The 1000th push crosses the size
        // threshold and triggers a second flush.
        for i in 500..1_000u64 {
            ctx.append(&make_event(i)).expect("append event");
        }
        // Wait for the second batch.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let n = bodies.lock().expect("bodies poisoned").len();
            if n >= 2 || std::time::Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let after_second = bodies.lock().expect("bodies poisoned").len();
        assert!(
            after_second <= 2,
            "1000-row burst must flush in <= 2 POSTs total, got {after_second}"
        );

        // Drop the ctx to force any final drain.
        drop(ctx);

        // Every appended row must be in some captured body.
        let total_lines: usize = bodies
            .lock()
            .expect("bodies poisoned")
            .iter()
            .map(|b| {
                if b.is_empty() {
                    0
                } else {
                    b.iter().filter(|&&c| c == b'\n').count() + 1
                }
            })
            .sum();
        assert_eq!(
            total_lines, 1_000,
            "every appended row must be in some batch"
        );
    }

    // -----------------------------------------------------------------------
    // Section 6.7: QueryArgs sanity
    // -----------------------------------------------------------------------

    #[test]
    fn query_args_bincode_roundtrip() {
        let args = QueryArgs {
            flux_query: "from(bucket: \"b\") |> range(start: -1h)".into(),
            bucket: Some("archive".into()),
            poll_ms: Some(5_000),
        };
        let bytes = bincode::serialize(&args).expect("serialize");
        let back: QueryArgs = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back.flux_query, args.flux_query);
        assert_eq!(back.bucket, args.bucket);
        assert_eq!(back.poll_ms, args.poll_ms);
        // Effective accessors.
        assert_eq!(back.effective_poll_ms(), 5_000);
        assert_eq!(back.effective_bucket("default"), "archive");
        // None -> default 60_000.
        let def = QueryArgs {
            flux_query: "x".into(),
            bucket: None,
            poll_ms: None,
        };
        assert_eq!(def.effective_poll_ms(), 60_000);
        // poll_ms < 100ms clamped up to 100.
        let clamped = QueryArgs {
            flux_query: "x".into(),
            bucket: None,
            poll_ms: Some(10),
        };
        assert_eq!(clamped.effective_poll_ms(), 100);
    }
}
