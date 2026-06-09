//! `query` Input adapter — `POST /api/v2/query` (Flux).
//!
//! ## Threading model
//!
//! 1. `QueryCtx::spawn()` decodes the per-stream config and
//!    builds a multi-thread tokio runtime shared via `Arc`.
//! 2. The worker thread enters the runtime and runs a polling
//!    loop: on each `poll_ms` tick it executes the Flux query
//!    and pushes result rows into a tokio mpsc channel.
//! 3. The FFI thread (`next`) calls
//!    `runtime.block_on(rx.recv())` to pull the next row. The
//!    `Arc<Runtime>` is shared between the FFI thread and the
//!    worker, so the FFI thread can safely call `block_on`.
//! 4. `close()` drops the ctx; the `Drop` impl drops the
//!    `Arc<Runtime>` (last ref) which terminates the worker.
//!
//! ## Output format
//!
//! Each Flux result row is serialised as a
//! [`FluxRow { table, columns: HashMap<String, String> }`]
//! and bincode-encoded as the `Event::payload`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bee_adapter::Event;
use serde::{Deserialize, Serialize};

use crate::config::{InfluxdbConfig, QueryArgs};
use crate::ratelimit::RateLimiter;
use crate::InfluxdbError;

/// One Flux result row. The Flux response is annotated CSV;
/// we flatten each row into a `(column_name, value_string)` map
/// plus the table index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FluxRow {
    /// Table index (0-based). Flux returns multiple "tables"
    /// in one response when the query has `union` or `pivot`.
    pub table: i32,
    /// Column name -> stringified value. Numbers are emitted
    /// as their `to_string` form, booleans as `"true"` /
    /// `"false"`, strings without surrounding quotes.
    pub columns: HashMap<String, String>,
}

impl FluxRow {
    /// Wrap a [`FluxRow`] in a `bee_adapter::Event` for the
    /// host. `timestamp` is the wall-clock time the row was
    /// produced (ms since epoch). `sequence` is 0 — the
    /// Compiler-side handler is responsible for sequencing.
    pub fn to_event(&self) -> Event {
        let payload = bincode::serialize(self).unwrap_or_default();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Event {
            timestamp: now_ms,
            sequence: 0,
            payload,
        }
    }
}

/// FFI ctx for the `query` adapter.
pub struct QueryCtx {
    /// The mpsc receiver the worker pushes rows into. The FFI
    /// `next` calls `recv` on this via the FFI thread's
    /// dedicated `block_on` runtime.
    rx: tokio::sync::mpsc::Receiver<FluxRow>,
    /// FFI thread's current-thread tokio runtime, used to
    /// `block_on(rx.recv())`. A separate runtime from the
    /// worker's (multi-thread) runtime so `block_on` from the
    /// FFI thread doesn't interfere with the polling loop.
    ffi_runtime: Arc<tokio::runtime::Runtime>,
    /// Worker thread handle. Joined on drop.
    worker: Option<std::thread::JoinHandle<()>>,
    /// Worker-side multi-thread runtime. Held so we can drop
    /// it on close, which terminates the polling loop.
    worker_runtime: Option<Arc<tokio::runtime::Runtime>>,
}

impl QueryCtx {
    /// Spawn the polling loop. The worker runs forever (until
    /// the runtime is dropped) and pushes rows into the
    /// channel. The first poll happens immediately, then every
    /// `poll_ms` thereafter.
    pub fn spawn(
        config: InfluxdbConfig,
        args: QueryArgs,
    ) -> Result<Self, InfluxdbError> {
        if config.url.is_empty() {
            return Err(InfluxdbError::Config("url is required".into()));
        }
        if config.token.is_empty() {
            return Err(InfluxdbError::Config("token is required".into()));
        }
        if config.org.is_empty() {
            return Err(InfluxdbError::Config("org is required".into()));
        }
        let effective_bucket = args.effective_bucket(&config.bucket).to_string();
        if effective_bucket.is_empty() {
            return Err(InfluxdbError::Config(
                "bucket is required (no Datasource default and no per-call override)".into(),
            ));
        }

        // Worker runtime: multi-thread, owns the polling loop.
        let worker_runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| InfluxdbError::Runtime(e.to_string()))?,
        );
        // FFI runtime: current-thread, used to `block_on` the
        // mpsc `recv` from the synchronous FFI thread.
        let ffi_runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| InfluxdbError::Runtime(e.to_string()))?,
        );

        // Bounded channel: 256 rows capacity. If the FFI
        // thread is slow, the worker applies backpressure.
        let (tx, rx) = tokio::sync::mpsc::channel::<FluxRow>(256);

        let worker_config = config.clone();
        let worker_args = args.clone();
        let worker_bucket = effective_bucket.clone();
        let worker_runtime_clone = Arc::clone(&worker_runtime);
        let worker_handle = std::thread::Builder::new()
            .name("influxdb-query".into())
            .spawn(move || {
                let _guard = worker_runtime_clone.enter();
                worker_runtime_clone.block_on(poll_loop(
                    worker_config,
                    worker_args,
                    worker_bucket,
                    tx,
                ));
            })
            .map_err(|e| InfluxdbError::Runtime(format!("spawn worker: {e}")))?;

        Ok(Self {
            rx,
            ffi_runtime,
            worker: Some(worker_handle),
            worker_runtime: Some(worker_runtime),
        })
    }

    /// Block on the next row. Returns `None` if the producer
    /// has closed the channel (worker dropped or runtime
    /// dropped).
    pub fn next_event(&mut self) -> Option<Event> {
        let rx = &mut self.rx;
        let row = self.ffi_runtime.block_on(async move { rx.recv().await })?;
        Some(row.to_event())
    }
}

impl Drop for QueryCtx {
    fn drop(&mut self) {
        // 1. Close the receiver so the worker's `tx.send` calls
        //    fail, then join the worker.
        // 2. Drop the worker runtime after the worker joins;
        //    the worker holds a clone so the runtime isn't
        //    freed until the worker thread exits.
        self.rx.close();
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
        self.worker_runtime = None;
    }
}

// ---------------------------------------------------------------------------
// Worker: polling loop
// ---------------------------------------------------------------------------

async fn poll_loop(
    config: InfluxdbConfig,
    args: QueryArgs,
    bucket: String,
    tx: tokio::sync::mpsc::Sender<FluxRow>,
) {
    let limiter = RateLimiter::new(config.rate_limit_per_sec);
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(config.timeout_ms))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("influxdb query: client build: {e}");
            return;
        }
    };
    let url = format!(
        "{}/api/v2/query?org={}",
        config.url.trim_end_matches('/'),
        urlencoding(&config.org),
    );
    let poll_interval = Duration::from_millis(args.effective_poll_ms());

    let mut ticker = tokio::time::interval(poll_interval);
    // The first tick fires immediately; that's the spec'd
    // behaviour (first poll on subscribe).
    loop {
        ticker.tick().await;
        limiter.wait().await;
        match execute_query(&client, &config, &url, &args.flux_query, &bucket).await {
            Ok(rows) => {
                for row in rows {
                    if tx.send(row).await.is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                log::warn!("influxdb query: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Query execution
// ---------------------------------------------------------------------------

/// Parse one Flux CSV response (the InfluxDB v2 query response
/// is annotated CSV: each chunk is `,result,table,_value,...`).
fn parse_flux_csv(body: &str) -> Vec<FluxRow> {
    // Annotated CSV layout:
    //   #group,false,false,true,...
    //   #datatype,string,long,...
    //   #default,_result,...
    //   ,result,table,_field,_value       <-- header
    //   ,,0,price,100.5
    //   ,,0,volume,200.0
    //   <blank>
    //   ,result,table,_field,_value       <-- next header
    //   ,,1,price,99.0
    let mut rows: Vec<FluxRow> = Vec::new();
    let mut header: Option<Vec<String>> = None;
    for raw_line in body.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        if header.is_none() {
            // Skip the empty leading cell (`,result,...`) if
            // present.
            let header_cells: &[&str] = if cells.first().map(|s| s.is_empty()).unwrap_or(false) {
                &cells[1..]
            } else {
                &cells[..]
            };
            header = Some(header_cells.iter().map(|s| s.to_string()).collect());
            continue;
        }
        let Some(hdr) = header.as_ref() else {
            continue;
        };
        // Data row: skip the leading two cells (the empty
        // marker and the `result` cell), then map the rest by
        // header index.
        let data_cells: Vec<&str> = if cells.len() >= 2 && cells[0].is_empty() {
            cells[2..].to_vec()
        } else {
            cells.clone()
        };
        let table_idx = cells
            .get(2)
            .and_then(|c| c.parse::<i32>().ok())
            .unwrap_or(0);
        let mut columns: HashMap<String, String> = HashMap::new();
        for (i, value) in data_cells.iter().enumerate() {
            if let Some(name) = hdr.get(i) {
                columns.insert(name.clone(), (*value).to_string());
            }
        }
        rows.push(FluxRow {
            table: table_idx,
            columns,
        });
    }
    rows
}

async fn execute_query(
    client: &reqwest::Client,
    config: &InfluxdbConfig,
    url: &str,
    flux_query: &str,
    bucket: &str,
) -> Result<Vec<FluxRow>, InfluxdbError> {
    // Naive bucket injection: if the query does not already
    // contain `from(bucket:`, we wrap it. The Compiler is
    // expected to inline the bucket in the query, so this is
    // a fallback.
    let rewritten = if flux_query.contains("from(bucket:") {
        flux_query.to_string()
    } else if flux_query.contains("from(") {
        flux_query.replace("from(", &format!("from(bucket: \"{bucket}\", "))
    } else {
        format!("from(bucket: \"{bucket}\") |> {flux_query}")
    };
    let body = format!(
        "org={}&query={}",
        urlencoding(&config.org),
        urlencoding(&rewritten),
    );
    let resp = client
        .post(url)
        .header("Authorization", format!("Token {}", config.token))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/csv")
        .body(body)
        .send()
        .await
        .map_err(|e| InfluxdbError::Http(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        let preview = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(512)
            .collect::<String>();
        return Err(InfluxdbError::Http(format!(
            "POST /api/v2/query status={status} body={preview:?}"
        )));
    }
    let csv = resp
        .text()
        .await
        .map_err(|e| InfluxdbError::Http(e.to_string()))?;
    Ok(parse_flux_csv(&csv))
}

/// Percent-encode an InfluxDB org or bucket name.
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
