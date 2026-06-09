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
