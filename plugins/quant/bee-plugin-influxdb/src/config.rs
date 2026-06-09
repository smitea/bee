//! Datasource-level and per-call config types for the influxdb
//! adapter.
//!
//! ## Layout
//!
//! - [`InfluxdbConfig`] — Datasource (connection-level) config.
//!   Registered once per Datasource with `bee datasource create`.
//!   Holds the URL, token, default org + bucket, timeout, and the
//!   tenant id (ADR-0010). The token is read from the bee secret
//!   store by the admin at registration time; it never appears in
//!   logs or error paths.
//! - [`WriteArgs`] — per-call args for the `write` Output method.
//!   The Compiler passes these to `open()` alongside the
//!   Datasource config (bundled into the FFI's single bincode
//!   blob). Specifies the measurement, the bucket override (if
//!   any), the tag / field column selectors, and the timestamp
//!   column name.
//! - [`QueryArgs`] — per-call args for the `query` Input method.
//!   The Flux query string, the bucket override (if any), and
//!   the polling cadence.

use serde::{Deserialize, Serialize};

/// Datasource-level config (ADR-0010). Connection-level only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfluxdbConfig {
    /// Base URL of the InfluxDB v2 instance (e.g.
    /// `"http://localhost:8086"`). Admin-supplied.
    pub url: String,
    /// InfluxDB API token. Sourced from the bee secret store; never
    /// logged, never included in any error message.
    pub token: String,
    /// InfluxDB organisation name (required).
    pub org: String,
    /// Default bucket. Per-call `bucket` args may override this.
    pub bucket: String,
    /// HTTP request timeout in milliseconds. Default 5000.
    pub timeout_ms: u64,
    /// Per-request rate limit (requests per second). Default 100.
    pub rate_limit_per_sec: u32,
    /// Flush size threshold (number of buffered line-protocol
    /// rows). Default 500.
    pub max_batch_size: usize,
    /// Flush time threshold in milliseconds (whichever of size or
    /// time fires first triggers a flush). Default 1000.
    pub flush_interval_ms: u64,
    /// Tenant id (uint16, 0 = global). ADR-0010.
    pub tenant: u16,
}

impl Default for InfluxdbConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8086".into(),
            token: String::new(),
            org: String::new(),
            bucket: String::new(),
            timeout_ms: 5_000,
            rate_limit_per_sec: 100,
            max_batch_size: 500,
            flush_interval_ms: 1_000,
            tenant: 0,
        }
    }
}

/// Per-call args for the `write` Output method. Mirrors the
/// `EMIT INTO influxdb.write(measurement, tag_cols, field_cols?, bucket?, timestamp_col?)`
/// SQL signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteArgs {
    /// Measurement name (e.g. `"klines"`, `"sentiment"`).
    pub measurement: String,
    /// Optional bucket override; if `None` the Datasource
    /// `bucket` is used.
    pub bucket: Option<String>,
    /// Columns to use as InfluxDB tags (string-valued).
    pub tag_cols: Vec<String>,
    /// Columns to use as InfluxDB fields (numeric). `None` means
    /// "all non-tag numeric columns" (the plugin picks them from
    /// the event's payload at runtime).
    pub field_cols: Option<Vec<String>>,
    /// Timestamp column name. Defaults to `"ts"`.
    pub timestamp_col: Option<String>,
}

impl WriteArgs {
    /// Effective timestamp column (the user-supplied one, or
    /// `"ts"` as the spec default).
    pub fn effective_timestamp_col(&self) -> &str {
        self.timestamp_col.as_deref().unwrap_or("ts")
    }

    /// Effective bucket: the per-call override, or the Datasource
    /// default.
    pub fn effective_bucket<'a>(&'a self, datasource_bucket: &'a str) -> &'a str {
        self.bucket.as_deref().unwrap_or(datasource_bucket)
    }
}

/// Per-call args for the `query` Input method. Mirrors the
/// `influxdb.query(flux_query, bucket?, poll_ms?)` SQL signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryArgs {
    /// Flux query string.
    pub flux_query: String,
    /// Optional bucket override; if `None` the Datasource
    /// `bucket` is used.
    pub bucket: Option<String>,
    /// Polling cadence in milliseconds. Default 60_000 (1 minute).
    pub poll_ms: Option<u64>,
}

impl QueryArgs {
    /// Effective poll interval. Clamped to a sane minimum (100ms)
    /// to avoid hammering the server on user error.
    pub fn effective_poll_ms(&self) -> u64 {
        self.poll_ms.unwrap_or(60_000).max(100)
    }

    /// Effective bucket: the per-call override, or the Datasource
    /// default.
    pub fn effective_bucket<'a>(&'a self, datasource_bucket: &'a str) -> &'a str {
        self.bucket.as_deref().unwrap_or(datasource_bucket)
    }
}
