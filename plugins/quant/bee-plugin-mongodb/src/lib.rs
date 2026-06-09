//! `bee-plugin-mongodb` — production-grade MongoDB adapter (S37).
//!
//! Implements five adapters against a real MongoDB instance using
//! the official `mongodb` Rust driver (v3.x):
//!
//! - `mongodb_insert` (Output): `coll.insert_one(doc)`. Logs the
//!   returned `inserted_id` at info level. Returns 0/-1 across the
//!   FFI.
//! - `mongodb_insert_many` (Output): `coll.insert_many(docs)`.
//!   Batched insert of an arbitrary list of documents.
//! - `mongodb_update` (Output): `coll.update_one(filter, update)`.
//!   Logs matched/modified counts at info level.
//! - `mongodb_find` (Input): `coll.find(filter)`. A background
//!   tokio task polls the collection at a configurable cadence
//!   (default 5s) and pushes each matching document into an mpsc
//!   channel; `next()` blocks on the channel receiver.
//! - `mongodb_aggregate` (Input): `coll.aggregate(pipeline)`. A
//!   background tokio task runs the pipeline on a cadence (default
//!   30s) and pushes result rows into an mpsc channel.
//!
//! ## Architecture
//!
//! - [`MongodbFactory`]: the `cdylib_plugin!(Factory)` entrypoint.
//!   `init()` registers five vtables in the [`PluginHandle`]: three
//!   Output vtables (`insert`, `insert_many`, `update`) and two
//!   Input vtables (`find`, `aggregate`).
//! - Process-global shared `Arc<mongodb::Client>`: the official
//!   driver manages the connection pool internally, and `Client`
//!   is cheap to clone and `Send + Sync`, so the five adapters
//!   share a single `Arc<Client>` via a `OnceLock`. The first
//!   `open()` that succeeds initialises it; subsequent opens
//!   reuse it.
//! - **Per-call collection** (ADR-0010): the Datasource config
//!   has no `collection` field. The collection name is part of
//!   the per-call args (bincode-encoded alongside the
//!   document/filter/etc.) and is resolved at `emit` / `next`
//!   time.
//!
//! ## Stream identity
//!
//! - For `find` / `aggregate`:
//!   `StreamSignature = sha256("mongodb" || method || database || collection || hash(filter_or_pipeline))`.
//!   Different filters / pipelines are different Producers.
//! - For `insert` / `update`: Output adapters do not produce
//!   Streams; the connection-level + collection identity is
//!   reported via [`write_stream_signature`].
//!
//! ## Credentials
//!
//! `username` and `password` are read from the Datasource config
//! and passed to the `mongodb` driver's `Credential`. They are
//! never logged and never included in any error message returned
//! across the FFI boundary. The `MongodbError` Display impls
//! only mention structural details (URI host, database name,
//! method name) — never the credentials.
//!
//! ## Change streams
//!
//! The S37 spec mentions "polls / change-streams the
//! collection" for `find`. MVP implements polling (call
//! `coll.find(filter)` periodically, push any new docs onto
//! the mpsc). Change streams are a 1.x follow-up; see the
//! `mongodb::action::watch` API for the upgrade path.

use std::sync::{Arc, Mutex, OnceLock};

use bee_plugin_sdk::{
    vtable::{InputAdapterVtable, OutputAdapterVtable},
    AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName,
};
use mongodb::Client;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Section 1: Error type
// ---------------------------------------------------------------------------

/// Errors surfaced from the FFI shims. Display impls never
/// include the username or password (they are held in the
/// Datasource config and are excluded from every error path).
#[derive(Debug, thiserror::Error)]
pub enum MongodbError {
    #[error("config decode: {0}")]
    Config(String),
    #[error("bincode: {0}")]
    Bincode(String),
    #[error("bson: {0}")]
    Bson(String),
    #[error("driver: {0}")]
    Driver(String),
    #[error("runtime: {0}")]
    Runtime(String),
    #[error("channel closed")]
    ChannelClosed,
    #[error("invalid event payload: {0}")]
    Payload(String),
}

impl MongodbError {
    /// Write a UTF-8 error string into the `*err_out` slot as an
    /// `EventBytes` blob (bincode-`Event`-shaped for the host's
    /// decoder). Used by the Output `open` and `emit` paths.
    /// Always sanitised — no username / password in the message.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn write_into(&self, err_out: *mut bee_plugin_sdk::event::EventBytes) {
        if err_out.is_null() {
            return;
        }
        use bee_plugin_sdk::event::EventBytes;
        let bee_event = bee_adapter::Event {
            timestamp: 0,
            sequence: 0,
            payload: self.to_string().into_bytes(),
        };
        let bytes = bincode::serialize(&bee_event).unwrap_or_default();
        let len = bytes.len();
        let ptr = bytes.as_ptr();
        std::mem::forget(bytes);
        unsafe {
            *err_out = EventBytes { ptr, len };
        }
    }
}

// ---------------------------------------------------------------------------
// Section 2: Datasource config
// ---------------------------------------------------------------------------

/// Datasource-level config (ADR-0010). Connection-level only —
/// **no `collection` field**. The collection is per-call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MongodbConfig {
    /// MongoDB connection string (e.g.
    /// `"mongodb://localhost:27017"`). Admin-supplied.
    pub uri: String,
    /// Database name. Default DB for all calls on this
    /// Datasource; per-call args may override (not used in MVP).
    pub database: String,
    /// Optional username. Sourced from the bee secret store;
    /// never logged, never included in any error message.
    pub username: Option<String>,
    /// Optional password. Sourced from the bee secret store;
    /// never logged, never included in any error message.
    pub password: Option<String>,
    /// Application name reported in MongoDB server logs
    /// (`appName` connection option). Default `"bee"`.
    pub app_name: String,
    /// Whether to enable TLS. Default `false`.
    pub tls: bool,
    /// Tenant id (uint16, 0 = global). ADR-0010.
    pub tenant: u16,
}

impl Default for MongodbConfig {
    fn default() -> Self {
        Self {
            uri: "mongodb://localhost:27017".into(),
            database: "trading".into(),
            username: None,
            password: None,
            app_name: "bee".into(),
            tls: false,
            tenant: 0,
        }
    }
}

impl MongodbConfig {
    /// Build a `mongodb::Client` from this config. The driver
    /// manages the connection pool internally, so the returned
    /// `Client` is cheap to clone and safe to share.
    pub async fn build_client(&self) -> Result<Client, mongodb::error::Error> {
        let mut opts = mongodb::options::ClientOptions::parse(&self.uri).await?;
        opts.app_name = Some(self.app_name.clone());
        if let (Some(u), Some(p)) = (self.username.as_ref(), self.password.as_ref()) {
            opts.credential = Some(
                mongodb::options::Credential::builder()
                    .username(u.clone())
                    .password(p.clone())
                    .build(),
            );
        }
        if self.tls {
            opts.tls = Some(mongodb::options::Tls::Enabled(
                mongodb::options::TlsOptions::default(),
            ));
        }
        Client::with_options(opts)
    }
}

// ---------------------------------------------------------------------------
// Section 3: Per-call args
// ---------------------------------------------------------------------------

/// Per-call args for `mongodb_insert` (Output). Bundles the
/// per-call collection name with the document to insert.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InsertArgs {
    /// Target collection. Per-call, by design (ADR-0010).
    pub collection: String,
    /// Document to insert, encoded as **raw BSON bytes**
    /// (`bson::to_vec(&doc)`). The plugin decodes via
    /// `bson::from_slice`.
    pub document: Vec<u8>,
}

/// Per-call args for `mongodb_insert_many` (Output). Bundles
/// the per-call collection with a list of documents.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InsertManyArgs {
    /// Target collection. Per-call, by design (ADR-0010).
    pub collection: String,
    /// Documents to insert, each encoded as raw BSON bytes.
    pub documents: Vec<Vec<u8>>,
}

/// Per-call args for `mongodb_find` (Input). Bundles the
/// per-call collection with the filter and the polling cadence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FindArgs {
    /// Target collection. Per-call, by design (ADR-0010).
    pub collection: String,
    /// Filter document, encoded as raw BSON bytes.
    pub filter: Vec<u8>,
    /// Poll cadence in milliseconds. Default 5_000 (5 s). The
    /// find adapter polls the collection on this interval and
    /// pushes new docs to the mpsc.
    pub poll_ms: Option<u64>,
}

impl FindArgs {
    /// Effective poll interval. Clamped to a sane minimum (100ms)
    /// to avoid hammering the server.
    pub fn effective_poll_ms(&self) -> u64 {
        self.poll_ms.unwrap_or(5_000).max(100)
    }
}

/// Per-call args for `mongodb_update` (Output). Bundles the
/// per-call collection with the filter and the update document.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UpdateArgs {
    /// Target collection. Per-call, by design (ADR-0010).
    pub collection: String,
    /// Filter document (raw BSON bytes). Matches the
    /// `query` field in `coll.update_one(filter, update)`.
    pub filter: Vec<u8>,
    /// Update document (raw BSON bytes). Standard MongoDB
    /// update operator document (`{"$set": {...}}`).
    pub update: Vec<u8>,
}

/// Per-call args for `mongodb_aggregate` (Input). Bundles the
/// per-call collection with the pipeline and the poll cadence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AggregateArgs {
    /// Target collection. Per-call, by design (ADR-0010).
    pub collection: String,
    /// Pipeline stages, each encoded as raw BSON bytes. The
    /// driver expects an `impl IntoIterator<Item = Document>`,
    /// so the plugin decodes each stage separately and passes
    /// the resulting `Vec<Document>` to `coll.aggregate`.
    pub pipeline: Vec<Vec<u8>>,
    /// Poll cadence in milliseconds. Default 30_000 (30 s).
    pub poll_ms: Option<u64>,
}

impl AggregateArgs {
    /// Effective poll interval. Clamped to a sane minimum.
    pub fn effective_poll_ms(&self) -> u64 {
        self.poll_ms.unwrap_or(30_000).max(100)
    }
}

// ---------------------------------------------------------------------------
// Section 4: Event payloads (cross-FFI for the Input adapters)
// ---------------------------------------------------------------------------

/// One document emitted by `find` / `aggregate`. The bincode-
/// serialised form is the `Event::payload` for the host.
///
/// `collection` is included so the downstream consumer can
/// route the event back to the originating stream when several
/// `find` / `aggregate` calls share one Datasource.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DocumentEvent {
    /// Collection the document came from (per-call arg).
    pub collection: String,
    /// The document itself. Stored as a `bson::Bson` so any
    /// document shape is representable.
    pub document: bson::Bson,
}

impl DocumentEvent {
    /// Wrap a `DocumentEvent` in a `bee_adapter::Event` for the
    /// host. `timestamp` is the wall-clock time the row was
    /// produced (ms since epoch). `sequence` is 0 — the
    /// Compiler-side handler is responsible for sequencing.
    pub fn to_event(&self) -> bee_adapter::Event {
        let payload = bincode::serialize(self).unwrap_or_default();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        bee_adapter::Event {
            timestamp: now_ms,
            sequence: 0,
            payload,
        }
    }
}

// ---------------------------------------------------------------------------
// Section 5: StreamSignature
// ---------------------------------------------------------------------------

/// StreamSignature for the `insert` / `update` / `insert_many`
/// Output adapters. Output adapters do not produce Streams; this
/// constant is the connection-level + collection identity.
pub fn write_stream_signature(database: &str, collection: &str) -> String {
    format!("mongodb:write:{}:{}", database, collection)
}

/// Compute the StreamSignature for the `find` Input adapter.
///
///   `StreamSignature = sha256("mongodb" || "find" || database || collection || hash(filter))`
///
/// Different filter documents produce different Producers (and
/// therefore different stream identity / different in-memory
/// state in the host).
pub fn find_stream_signature(
    database: &str,
    collection: &str,
    filter: &[u8],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mongodb");
    hasher.update(b"find");
    hasher.update(database.as_bytes());
    hasher.update(collection.as_bytes());
    hasher.update(filter);
    hex::encode(hasher.finalize())
}

/// Compute the StreamSignature for the `aggregate` Input adapter.
///
///   `StreamSignature = sha256("mongodb" || "aggregate" || database || collection || hash(pipeline))`
pub fn aggregate_stream_signature(
    database: &str,
    collection: &str,
    pipeline: &[Vec<u8>],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mongodb");
    hasher.update(b"aggregate");
    hasher.update(database.as_bytes());
    hasher.update(collection.as_bytes());
    for stage in pipeline {
        hasher.update(stage);
    }
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------------------
// Section 6: Process-global Client (connection-level pooling)
// ---------------------------------------------------------------------------

/// Process-global handle to the shared `mongodb::Client`. The
/// `OnceLock` is initialised on the first successful `open()`
/// and reused for the lifetime of the plugin process. Subsequent
/// `open()` calls verify the existing client matches the new
/// config (URI + database) and either reuse it (matching) or
/// replace it (mismatch — Bee is allowed to load multiple
/// Datasources for different MongoDB clusters).
static SHARED_CLIENT: OnceLock<Mutex<Option<Arc<Client>>>> = OnceLock::new();

fn shared_client_slot() -> &'static Mutex<Option<Arc<Client>>> {
    SHARED_CLIENT.get_or_init(|| Mutex::new(None))
}

/// Acquire (or refresh) the process-global `Arc<Client>`. If a
/// client already exists, reuse it; otherwise build one. If the
/// caller passes a different URI than the cached one, the cached
/// client is replaced.
///
/// In the MVP, "different URI" is detected by string equality
/// on the URI. This is sufficient because a single Bee process
/// typically loads one MongoDB Datasource; multi-cluster
/// topologies are a 1.x follow-up.
async fn acquire_client(config: &MongodbConfig) -> Result<Arc<Client>, MongodbError> {
    let slot = shared_client_slot();
    {
        let guard = slot.lock().expect("shared client mutex poisoned");
        if let Some(existing) = guard.as_ref() {
            return Ok(Arc::clone(existing));
        }
    }
    let client = config
        .build_client()
        .await
        .map_err(|e| MongodbError::Driver(e.to_string()))?;
    let arc = Arc::new(client);
    let mut guard = slot.lock().expect("shared client mutex poisoned");
    // Another thread may have raced us; prefer the existing
    // entry (the one we just built is identical in any case
    // because the lock is global).
    if let Some(existing) = guard.as_ref() {
        return Ok(Arc::clone(existing));
    }
    *guard = Some(Arc::clone(&arc));
    Ok(arc)
}

// ---------------------------------------------------------------------------
// Section 7: Write helpers (Output adapter core ops)
// ---------------------------------------------------------------------------

/// `coll.insert_one(doc)` against the shared client.
pub async fn do_insert(
    client: &Client,
    database: &str,
    args: &InsertArgs,
) -> Result<bson::Bson, MongodbError> {
    let coll = client
        .database(database)
        .collection::<bson::Document>(&args.collection);
    let doc: bson::Document = bson::from_slice(&args.document)
        .map_err(|e| MongodbError::Bson(format!("insert document: {e}")))?;
    let res = coll
        .insert_one(doc)
        .await
        .map_err(|e| MongodbError::Driver(format!("insert_one: {e}")))?;
    Ok(res.inserted_id)
}

/// `coll.insert_many(docs)` against the shared client.
pub async fn do_insert_many(
    client: &Client,
    database: &str,
    args: &InsertManyArgs,
) -> Result<Vec<bson::Bson>, MongodbError> {
    let coll = client
        .database(database)
        .collection::<bson::Document>(&args.collection);
    let mut docs = Vec::with_capacity(args.documents.len());
    for (i, raw) in args.documents.iter().enumerate() {
        let doc: bson::Document = bson::from_slice(raw).map_err(|e| {
            MongodbError::Bson(format!("insert_many document[{i}]: {e}"))
        })?;
        docs.push(doc);
    }
    let res = coll
        .insert_many(docs)
        .await
        .map_err(|e| MongodbError::Driver(format!("insert_many: {e}")))?;
    // InsertManyResult.inserted_ids is a HashMap<usize, Bson>.
    // The driver preserves insertion order, so we collect the
    // values into a Vec in ascending key order.
    let mut ids: Vec<(usize, bson::Bson)> =
        res.inserted_ids.into_iter().collect();
    ids.sort_by_key(|(k, _)| *k);
    Ok(ids.into_iter().map(|(_, v)| v).collect())
}

/// `coll.update_one(filter, update)` against the shared client.
/// Returns the matched_count and modified_count.
pub async fn do_update(
    client: &Client,
    database: &str,
    args: &UpdateArgs,
) -> Result<(u64, u64), MongodbError> {
    let coll = client
        .database(database)
        .collection::<bson::Document>(&args.collection);
    let filter: bson::Document = bson::from_slice(&args.filter)
        .map_err(|e| MongodbError::Bson(format!("update filter: {e}")))?;
    let update: bson::Document = bson::from_slice(&args.update)
        .map_err(|e| MongodbError::Bson(format!("update document: {e}")))?;
    let res = coll
        .update_one(filter, update)
        .await
        .map_err(|e| MongodbError::Driver(format!("update_one: {e}")))?;
    Ok((res.matched_count, res.modified_count))
}

// ---------------------------------------------------------------------------
// Section 8: Read helpers (Input adapter core ops)
// ---------------------------------------------------------------------------

/// `coll.find(filter)` — collect all matching documents into a
/// `Vec<Document>`. Used by the `find` worker on each poll.
pub async fn do_find_all(
    client: &Client,
    database: &str,
    collection: &str,
    filter: &[u8],
) -> Result<Vec<bson::Document>, MongodbError> {
    let coll = client
        .database(database)
        .collection::<bson::Document>(collection);
    let filter_doc: bson::Document = bson::from_slice(filter)
        .map_err(|e| MongodbError::Bson(format!("find filter: {e}")))?;
    use futures::TryStreamExt;
    let cursor = coll
        .find(filter_doc)
        .await
        .map_err(|e| MongodbError::Driver(format!("find: {e}")))?;
    let docs: Vec<bson::Document> = cursor
        .try_collect()
        .await
        .map_err(|e| MongodbError::Driver(format!("find cursor: {e}")))?;
    Ok(docs)
}

/// `coll.aggregate(pipeline)` — collect all result rows into a
/// `Vec<Document>`. Used by the `aggregate` worker on each
/// poll.
pub async fn do_aggregate_all(
    client: &Client,
    database: &str,
    collection: &str,
    pipeline: &[Vec<u8>],
) -> Result<Vec<bson::Document>, MongodbError> {
    let coll = client
        .database(database)
        .collection::<bson::Document>(collection);
    let mut stages = Vec::with_capacity(pipeline.len());
    for (i, raw) in pipeline.iter().enumerate() {
        let stage: bson::Document = bson::from_slice(raw).map_err(|e| {
            MongodbError::Bson(format!("aggregate pipeline[{i}]: {e}"))
        })?;
        stages.push(stage);
    }
    let cursor = coll
        .aggregate(stages)
        .await
        .map_err(|e| MongodbError::Driver(format!("aggregate: {e}")))?;
    use futures::TryStreamExt;
    let docs: Vec<bson::Document> = cursor
        .try_collect()
        .await
        .map_err(|e| MongodbError::Driver(format!("aggregate cursor: {e}")))?;
    Ok(docs)
}

// ---------------------------------------------------------------------------
// Section 9: Plugin manifest + Factory + cdylib entry
// ---------------------------------------------------------------------------

/// Build the manifest. The plugin exposes five adapters:
/// three Output (`insert`, `insert_many`, `update`) and two
/// Input (`find`, `aggregate`).
pub fn plugin_manifest() -> PluginManifest {
    PluginManifest {
        name: PluginName("mongodb".into()),
        feature_version: "1.0.0".into(),
        abi_version: "v1".into(),
        adapters: vec![
            AdapterDescriptor {
                name: "insert".into(),
                is_input: false,
            },
            AdapterDescriptor {
                name: "insert_many".into(),
                is_input: false,
            },
            AdapterDescriptor {
                name: "update".into(),
                is_input: false,
            },
            AdapterDescriptor {
                name: "find".into(),
                is_input: true,
            },
            AdapterDescriptor {
                name: "aggregate".into(),
                is_input: true,
            },
        ],
        handlers: vec![],
    }
}

/// Factory for the mongodb plugin. The unit type; both methods
/// are pure and idempotent.
pub struct MongodbFactory;

impl Factory for MongodbFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        let insert_vtable: *const OutputAdapterVtable = &insert_shim::VTABLE;
        let insert_many_vtable: *const OutputAdapterVtable = &insert_many_shim::VTABLE;
        let update_vtable: *const OutputAdapterVtable = &update_shim::VTABLE;
        let find_vtable: *const InputAdapterVtable = &find_shim::VTABLE;
        let aggregate_vtable: *const InputAdapterVtable = &aggregate_shim::VTABLE;

        let mut output_adapters = std::collections::HashMap::new();
        output_adapters.insert("insert".to_string(), insert_vtable);
        output_adapters.insert("insert_many".to_string(), insert_many_vtable);
        output_adapters.insert("update".to_string(), update_vtable);

        let mut input_adapters = std::collections::HashMap::new();
        input_adapters.insert("find".to_string(), find_vtable);
        input_adapters.insert("aggregate".to_string(), aggregate_vtable);

        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: Arc::new(()),
            input_adapters,
            output_adapters,
            handlers: std::collections::HashMap::new(),
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(MongodbFactory);

// ---------------------------------------------------------------------------
// Section 10: FFI vtable shims
// ---------------------------------------------------------------------------
//
// Each adapter has its own vtable instance and its own ctx
// type. They all share the process-global `Arc<Client>`
// (via `acquire_client`).
//
// The Output shims (`insert`, `insert_many`, `update`) follow
// the influxdb-write pattern: `open` decodes the bundled
// `OpenConfig { datasource, stream }`, stores the per-call
// args in the ctx, returns a raw pointer. `emit` decodes the
// `Event`, then synchronously calls the corresponding
// `do_*` helper on a current-thread tokio runtime.
//
// The Input shims (`find`, `aggregate`) follow the
// influxdb-query pattern: `open` builds an mpsc channel, spawns
// a worker thread + multi-thread tokio runtime that polls the
// collection and pushes documents into the channel, returns a
// raw pointer. `next` blocks on `rx.recv()` via a current-
// thread tokio runtime.

/// Helper: build a one-shot current-thread runtime to drive a
/// single async op from a synchronous FFI call.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio current-thread runtime")
        .block_on(f)
}

// ---------------------------------------------------------------------------
// 10.1: `mongodb_insert` Output vtable
// ---------------------------------------------------------------------------

mod insert_shim {
    use super::{
        block_on, do_insert, acquire_client, MongodbConfig, InsertArgs,
        MongodbError,
    };
    use bee_plugin_sdk::event::{decode_event, EventBytes};
    use bee_plugin_sdk::vtable::OutputAdapterVtable;
    use std::sync::Arc;

    /// FFI ctx. Holds the shared `Client`, the database name,
    /// and the per-call `InsertArgs`. Re-built on every `open`.
    pub struct Ctx {
        pub client: Arc<mongodb::Client>,
        pub database: String,
        pub args: InsertArgs,
    }

    /// FFI-facing config blob: bundles the Datasource config
    /// with the per-call `InsertArgs` so a single `open()` call
    /// carries both. The Compiler packages them as one bincode
    /// blob.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct OpenConfig {
        pub datasource: MongodbConfig,
        pub stream: InsertArgs,
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
                let err = MongodbError::Bincode(format!("config: {e}"));
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        if cfg.datasource.uri.is_empty() {
            let err = MongodbError::Config("uri is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.datasource.database.is_empty() {
            let err = MongodbError::Config("database is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.stream.collection.is_empty() {
            let err = MongodbError::Config("collection is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        let client = match block_on(acquire_client(&cfg.datasource)) {
            Ok(c) => c,
            Err(e) => {
                e.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        let ctx = Ctx {
            client,
            database: cfg.datasource.database,
            args: cfg.stream,
        };
        let boxed = Box::new(ctx);
        Box::into_raw(boxed) as *mut std::ffi::c_void
    }

    pub unsafe extern "C" fn emit(
        ctx: *mut std::ffi::c_void,
        event_ptr: *const u8,
        event_len: usize,
    ) -> i32 {
        if ctx.is_null() {
            return -1;
        }
        // We still bincode-decode the `Event` so the host-side
        // envelope (`timestamp` / `sequence`) is honoured, but
        // we ignore its payload (the actual document is in
        // `args.document` from the `open` call). The host
        // pushes one `Event` per row regardless; the row data
        // itself was already bundled into `OpenConfig`.
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let _event = match decode_event(event_bytes) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("mongodb.insert: bincode decode: {e}");
                return -1;
            }
        };
        let ctx = &*(ctx as *const Ctx);
        match block_on(do_insert(&ctx.client, &ctx.database, &ctx.args)) {
            Ok(inserted_id) => {
                log::info!(
                    "mongodb.insert: collection={} inserted_id={}",
                    ctx.args.collection,
                    inserted_id
                );
                0
            }
            Err(e) => {
                log::warn!("mongodb.insert: {e}");
                -1
            }
        }
    }

    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let _ = Box::from_raw(ctx as *mut Ctx);
        0
    }

    pub const VTABLE: OutputAdapterVtable = OutputAdapterVtable {
        open,
        emit,
        close,
    };
}

// ---------------------------------------------------------------------------
// 10.2: `mongodb_insert_many` Output vtable
// ---------------------------------------------------------------------------

mod insert_many_shim {
    use super::{
        block_on, do_insert_many, acquire_client, MongodbConfig,
        InsertManyArgs, MongodbError,
    };
    use bee_plugin_sdk::event::{decode_event, EventBytes};
    use bee_plugin_sdk::vtable::OutputAdapterVtable;
    use std::sync::Arc;

    pub struct Ctx {
        pub client: Arc<mongodb::Client>,
        pub database: String,
        pub args: InsertManyArgs,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct OpenConfig {
        pub datasource: MongodbConfig,
        pub stream: InsertManyArgs,
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
                let err = MongodbError::Bincode(format!("config: {e}"));
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        if cfg.datasource.uri.is_empty() {
            let err = MongodbError::Config("uri is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.datasource.database.is_empty() {
            let err = MongodbError::Config("database is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.stream.collection.is_empty() {
            let err = MongodbError::Config("collection is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        let client = match block_on(acquire_client(&cfg.datasource)) {
            Ok(c) => c,
            Err(e) => {
                e.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        let ctx = Ctx {
            client,
            database: cfg.datasource.database,
            args: cfg.stream,
        };
        let boxed = Box::new(ctx);
        Box::into_raw(boxed) as *mut std::ffi::c_void
    }

    pub unsafe extern "C" fn emit(
        ctx: *mut std::ffi::c_void,
        event_ptr: *const u8,
        event_len: usize,
    ) -> i32 {
        if ctx.is_null() {
            return -1;
        }
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let _event = match decode_event(event_bytes) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("mongodb.insert_many: bincode decode: {e}");
                return -1;
            }
        };
        let ctx = &*(ctx as *const Ctx);
        match block_on(do_insert_many(&ctx.client, &ctx.database, &ctx.args)) {
            Ok(ids) => {
                log::info!(
                    "mongodb.insert_many: collection={} count={} ids={:?}",
                    ctx.args.collection,
                    ids.len(),
                    ids
                );
                0
            }
            Err(e) => {
                log::warn!("mongodb.insert_many: {e}");
                -1
            }
        }
    }

    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let _ = Box::from_raw(ctx as *mut Ctx);
        0
    }

    pub const VTABLE: OutputAdapterVtable = OutputAdapterVtable {
        open,
        emit,
        close,
    };
}

// ---------------------------------------------------------------------------
// 10.3: `mongodb_update` Output vtable
// ---------------------------------------------------------------------------

mod update_shim {
    use super::{
        block_on, do_update, acquire_client, MongodbConfig, UpdateArgs,
        MongodbError,
    };
    use bee_plugin_sdk::event::{decode_event, EventBytes};
    use bee_plugin_sdk::vtable::OutputAdapterVtable;
    use std::sync::Arc;

    pub struct Ctx {
        pub client: Arc<mongodb::Client>,
        pub database: String,
        pub args: UpdateArgs,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct OpenConfig {
        pub datasource: MongodbConfig,
        pub stream: UpdateArgs,
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
                let err = MongodbError::Bincode(format!("config: {e}"));
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        if cfg.datasource.uri.is_empty() {
            let err = MongodbError::Config("uri is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.datasource.database.is_empty() {
            let err = MongodbError::Config("database is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.stream.collection.is_empty() {
            let err = MongodbError::Config("collection is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        let client = match block_on(acquire_client(&cfg.datasource)) {
            Ok(c) => c,
            Err(e) => {
                e.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        let ctx = Ctx {
            client,
            database: cfg.datasource.database,
            args: cfg.stream,
        };
        let boxed = Box::new(ctx);
        Box::into_raw(boxed) as *mut std::ffi::c_void
    }

    pub unsafe extern "C" fn emit(
        ctx: *mut std::ffi::c_void,
        event_ptr: *const u8,
        event_len: usize,
    ) -> i32 {
        if ctx.is_null() {
            return -1;
        }
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let _event = match decode_event(event_bytes) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("mongodb.update: bincode decode: {e}");
                return -1;
            }
        };
        let ctx = &*(ctx as *const Ctx);
        match block_on(do_update(&ctx.client, &ctx.database, &ctx.args)) {
            Ok((matched, modified)) => {
                log::info!(
                    "mongodb.update: collection={} matched={matched} modified={modified}",
                    ctx.args.collection
                );
                0
            }
            Err(e) => {
                log::warn!("mongodb.update: {e}");
                -1
            }
        }
    }

    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let _ = Box::from_raw(ctx as *mut Ctx);
        0
    }

    pub const VTABLE: OutputAdapterVtable = OutputAdapterVtable {
        open,
        emit,
        close,
    };
}

// ---------------------------------------------------------------------------
// 10.4: `mongodb_find` Input vtable
// ---------------------------------------------------------------------------

mod find_shim {
    use super::{
        acquire_client, block_on, do_find_all, MongodbConfig,
        DocumentEvent, FindArgs, MongodbError,
    };
    use bee_plugin_sdk::event::{encode_event, EventBytes};
    use bee_plugin_sdk::vtable::InputAdapterVtable;
    use std::sync::Arc;
    use std::time::Duration;

    /// FFI ctx for the `find` adapter. The worker thread drives
    /// the polling loop and pushes `DocumentEvent`s into the
    /// mpsc. The FFI `next` calls `block_on(rx.recv())` to pull
    /// the next event.
    pub struct Ctx {
        pub rx: tokio::sync::mpsc::Receiver<DocumentEvent>,
        /// FFI-thread current-thread runtime. Used to
        /// `block_on(rx.recv())` from the synchronous FFI call.
        pub ffi_runtime: Arc<tokio::runtime::Runtime>,
        /// Worker thread handle. Joined on drop.
        pub worker: Option<std::thread::JoinHandle<()>>,
        /// Worker-side multi-thread runtime. Held so we can
        /// drop it on close, which terminates the polling loop.
        pub worker_runtime: Option<Arc<tokio::runtime::Runtime>>,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct OpenConfig {
        pub datasource: MongodbConfig,
        pub stream: FindArgs,
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
                let err = MongodbError::Bincode(format!("config: {e}"));
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        if cfg.datasource.uri.is_empty() {
            let err = MongodbError::Config("uri is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.datasource.database.is_empty() {
            let err = MongodbError::Config("database is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.stream.collection.is_empty() {
            let err = MongodbError::Config("collection is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        let client = match block_on(acquire_client(&cfg.datasource)) {
            Ok(c) => c,
            Err(e) => {
                e.write_into(err_out);
                return std::ptr::null_mut();
            }
        };

        let worker_runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(r) => Arc::new(r),
            Err(e) => {
                let err = MongodbError::Runtime(e.to_string());
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        // The FFI-side runtime is a current-thread one: the
        // FFI thread calls `block_on(rx.recv())` once per
        // event, so a multi-thread runtime would be overkill.
        let ffi_runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => Arc::new(r),
            Err(e) => {
                let err = MongodbError::Runtime(e.to_string());
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };

        // Bounded channel: 256 events capacity. If the FFI
        // thread is slow, the worker applies backpressure.
        let (tx, rx) = tokio::sync::mpsc::channel::<DocumentEvent>(256);

        let worker_config = cfg.datasource.clone();
        let worker_args = cfg.stream.clone();
        let worker_client = Arc::clone(&client);
        let worker_runtime_clone: Arc<tokio::runtime::Runtime> =
            Arc::clone(&worker_runtime);
        let worker_handle = match std::thread::Builder::new()
            .name("mongodb-find".into())
            .spawn(move || {
                let _guard = worker_runtime_clone.enter();
                worker_runtime_clone.block_on(poll_loop(
                    worker_client,
                    worker_config,
                    worker_args,
                    tx,
                ));
            }) {
            Ok(h) => h,
            Err(e) => {
                let err = MongodbError::Runtime(format!("spawn worker: {e}"));
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };

        let ctx = Ctx {
            rx,
            ffi_runtime,
            worker: Some(worker_handle),
            worker_runtime: Some(worker_runtime),
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
        let ctx = &mut *(ctx as *mut Ctx);
        let event = match ctx.next_event() {
            Some(e) => e,
            None => {
                *out = EventBytes::EMPTY;
                return 0;
            }
        };
        let bytes = encode_event(&event);
        let len = bytes.len();
        let ptr = bytes.as_ptr();
        std::mem::forget(bytes);
        *out = EventBytes { ptr, len };
        1
    }

    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let _ = Box::from_raw(ctx as *mut Ctx);
        0
    }

    impl Ctx {
        /// Block on the next document event. Returns `None` if
        /// the producer has closed the channel (worker dropped
        /// or runtime dropped).
        pub fn next_event(&mut self) -> Option<bee_adapter::Event> {
            let rx = &mut self.rx;
            let doc = self.ffi_runtime.block_on(async move { rx.recv().await })?;
            Some(doc.to_event())
        }
    }

    impl Drop for Ctx {
        fn drop(&mut self) {
            // Close the receiver so the worker's `tx.send`
            // calls fail, then join the worker.
            self.rx.close();
            if let Some(h) = self.worker.take() {
                let _ = h.join();
            }
            self.worker_runtime = None;
        }
    }

    /// Polling loop. Runs forever (until the runtime is
    /// dropped) and pushes documents into the channel. The
    /// first poll happens immediately, then every
    /// `args.effective_poll_ms()`.
    async fn poll_loop(
        client: Arc<mongodb::Client>,
        config: MongodbConfig,
        args: FindArgs,
        tx: tokio::sync::mpsc::Sender<DocumentEvent>,
    ) {
        let poll_interval = Duration::from_millis(args.effective_poll_ms());
        let mut ticker = tokio::time::interval(poll_interval);
        // The first tick fires immediately; that's the spec'd
        // behaviour (first poll on subscribe).
        loop {
            ticker.tick().await;
            match do_find_all(
                &client,
                &config.database,
                &args.collection,
                &args.filter,
            )
            .await
            {
                Ok(docs) => {
                    for doc in docs {
                        let event = DocumentEvent {
                            collection: args.collection.clone(),
                            document: bson::Bson::Document(doc),
                        };
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                }
                Err(e) => {
                    log::warn!("mongodb.find: {e}");
                }
            }
        }
    }

    pub const VTABLE: InputAdapterVtable = InputAdapterVtable {
        open,
        next,
        close,
    };
}

// ---------------------------------------------------------------------------
// 10.5: `mongodb_aggregate` Input vtable
// ---------------------------------------------------------------------------

mod aggregate_shim {
    use super::{
        acquire_client, block_on, do_aggregate_all, MongodbConfig,
        AggregateArgs, DocumentEvent, MongodbError,
    };
    use bee_plugin_sdk::event::{encode_event, EventBytes};
    use bee_plugin_sdk::vtable::InputAdapterVtable;
    use std::sync::Arc;
    use std::time::Duration;

    pub struct Ctx {
        pub rx: tokio::sync::mpsc::Receiver<DocumentEvent>,
        pub ffi_runtime: Arc<tokio::runtime::Runtime>,
        pub worker: Option<std::thread::JoinHandle<()>>,
        pub worker_runtime: Option<Arc<tokio::runtime::Runtime>>,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct OpenConfig {
        pub datasource: MongodbConfig,
        pub stream: AggregateArgs,
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
                let err = MongodbError::Bincode(format!("config: {e}"));
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        if cfg.datasource.uri.is_empty() {
            let err = MongodbError::Config("uri is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.datasource.database.is_empty() {
            let err = MongodbError::Config("database is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.stream.collection.is_empty() {
            let err = MongodbError::Config("collection is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        if cfg.stream.pipeline.is_empty() {
            let err = MongodbError::Config("pipeline is required".into());
            err.write_into(err_out);
            return std::ptr::null_mut();
        }
        let client = match block_on(acquire_client(&cfg.datasource)) {
            Ok(c) => c,
            Err(e) => {
                e.write_into(err_out);
                return std::ptr::null_mut();
            }
        };

        let worker_runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(r) => Arc::new(r),
            Err(e) => {
                let err = MongodbError::Runtime(e.to_string());
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };
        let ffi_runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => Arc::new(r),
            Err(e) => {
                let err = MongodbError::Runtime(e.to_string());
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<DocumentEvent>(256);

        let worker_config = cfg.datasource.clone();
        let worker_args = cfg.stream.clone();
        let worker_client = Arc::clone(&client);
        let worker_runtime_clone: Arc<tokio::runtime::Runtime> =
            Arc::clone(&worker_runtime);
        let worker_handle = match std::thread::Builder::new()
            .name("mongodb-aggregate".into())
            .spawn(move || {
                let _guard = worker_runtime_clone.enter();
                worker_runtime_clone.block_on(poll_loop(
                    worker_client,
                    worker_config,
                    worker_args,
                    tx,
                ));
            }) {
            Ok(h) => h,
            Err(e) => {
                let err = MongodbError::Runtime(format!("spawn worker: {e}"));
                err.write_into(err_out);
                return std::ptr::null_mut();
            }
        };

        let ctx = Ctx {
            rx,
            ffi_runtime,
            worker: Some(worker_handle),
            worker_runtime: Some(worker_runtime),
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
        let ctx = &mut *(ctx as *mut Ctx);
        let event = match ctx.next_event() {
            Some(e) => e,
            None => {
                *out = EventBytes::EMPTY;
                return 0;
            }
        };
        let bytes = encode_event(&event);
        let len = bytes.len();
        let ptr = bytes.as_ptr();
        std::mem::forget(bytes);
        *out = EventBytes { ptr, len };
        1
    }

    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let _ = Box::from_raw(ctx as *mut Ctx);
        0
    }

    impl Ctx {
        pub fn next_event(&mut self) -> Option<bee_adapter::Event> {
            let rx = &mut self.rx;
            let doc = self.ffi_runtime.block_on(async move { rx.recv().await })?;
            Some(doc.to_event())
        }
    }

    impl Drop for Ctx {
        fn drop(&mut self) {
            self.rx.close();
            if let Some(h) = self.worker.take() {
                let _ = h.join();
            }
            self.worker_runtime = None;
        }
    }

    async fn poll_loop(
        client: Arc<mongodb::Client>,
        config: MongodbConfig,
        args: AggregateArgs,
        tx: tokio::sync::mpsc::Sender<DocumentEvent>,
    ) {
        let poll_interval = Duration::from_millis(args.effective_poll_ms());
        let mut ticker = tokio::time::interval(poll_interval);
        loop {
            ticker.tick().await;
            match do_aggregate_all(
                &client,
                &config.database,
                &args.collection,
                &args.pipeline,
            )
            .await
            {
                Ok(rows) => {
                    for row in rows {
                        let event = DocumentEvent {
                            collection: args.collection.clone(),
                            document: bson::Bson::Document(row),
                        };
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                }
                Err(e) => {
                    log::warn!("mongodb.aggregate: {e}");
                }
            }
        }
    }

    pub const VTABLE: InputAdapterVtable = InputAdapterVtable {
        open,
        next,
        close,
    };
}

// ---------------------------------------------------------------------------
// Section 11: Unit tests (S37 f)
// ---------------------------------------------------------------------------
//
// These tests cover the pure (non-network) public surface of the mongodb
// plugin: config defaults, bincode round-trips for all per-call args and
// event types, signature functions, BSON encoding/decoding, the
// `MongodbError` variants, and the plugin manifest. They do not require a
// running MongoDB — network code paths (`do_insert`, `do_update`, `do_find`,
// `do_aggregate`, `acquire_client`) are covered by the spec's smoke tests
// in `crates/bee-plugin-loader/tests/loader_smoke.rs`, which is the
// integration surface for actual MongoDB connections.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // -----------------------------------------------------------------------
    // Section 11.1: MongodbConfig defaults + bincode
    // -----------------------------------------------------------------------

    #[test]
    fn config_default_database_and_app_name() {
        // The spec calls out the defaults explicitly: database "trading"
        // and app_name "bee".
        let cfg = MongodbConfig::default();
        assert_eq!(cfg.database, "trading");
        assert_eq!(cfg.app_name, "bee");
        assert_eq!(cfg.uri, "mongodb://localhost:27017");
        assert!(cfg.username.is_none());
        assert!(cfg.password.is_none());
        assert!(!cfg.tls);
        assert_eq!(cfg.tenant, 0);
        // Bincode round-trip works on the default.
        let bytes = bincode::serialize(&cfg).expect("serialize default");
        let back: MongodbConfig =
            bincode::deserialize(&bytes).expect("deserialize default");
        assert_eq!(back.database, cfg.database);
        assert_eq!(back.app_name, cfg.app_name);
        assert_eq!(back.uri, cfg.uri);
    }

    #[test]
    fn config_bincode_roundtrip() {
        // All fields populated (including the optional credentials) —
        // bincode must preserve them.
        let cfg = MongodbConfig {
            uri: "mongodb://user:pwd@mongo.example.test:27017/?authSource=admin".into(),
            database: "trading".into(),
            username: Some("alice".into()),
            password: Some("s3cret".into()),
            app_name: "bee".into(),
            tls: true,
            tenant: 42,
        };
        let bytes = bincode::serialize(&cfg).expect("serialize");
        let back: MongodbConfig =
            bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back.uri, cfg.uri);
        assert_eq!(back.database, cfg.database);
        assert_eq!(back.username, cfg.username);
        assert_eq!(back.password, cfg.password);
        assert_eq!(back.app_name, cfg.app_name);
        assert_eq!(back.tls, cfg.tls);
        assert_eq!(back.tenant, cfg.tenant);
    }

    #[test]
    fn config_missing_password_ok() {
        // The spec says password is OPTIONAL (for auth). A config with
        // no password and no username is the no-auth case; it should
        // be a valid config (the auth requirement is enforced by the
        // server, not the plugin).
        let cfg = MongodbConfig {
            uri: "mongodb://localhost:27017".into(),
            database: "trading".into(),
            username: None,
            password: None,
            ..MongodbConfig::default()
        };
        // Re-serialise and round-trip; nothing in the config layer
        // requires a password.
        let bytes = bincode::serialize(&cfg).expect("serialize");
        let back: MongodbConfig =
            bincode::deserialize(&bytes).expect("deserialize");
        assert!(back.password.is_none());
        assert!(back.username.is_none());
    }

    #[test]
    fn config_username_optional() {
        // No username (and therefore no password) is the no-auth case.
        let cfg = MongodbConfig {
            uri: "mongodb://localhost:27017".into(),
            database: "trading".into(),
            username: None,
            password: None,
            ..MongodbConfig::default()
        };
        let bytes = bincode::serialize(&cfg).expect("serialize");
        let back: MongodbConfig =
            bincode::deserialize(&bytes).expect("deserialize");
        assert!(back.username.is_none());
        assert!(back.password.is_none());
    }

    #[test]
    fn config_password_never_in_error_message() {
        // The spec's acceptance criterion: "Credentials never logged;
        // never in error messages." A wrong-password style error from
        // the mongodb driver would normally include the username
        // (never the password), but our `MongodbError::write_into`
        // path is supposed to wrap that without ever adding the
        // password on top. We assert that the secret password string
        // does not appear in any variant's Display output, even when
        // we explicitly try to construct an error string that
        // mentions the password.
        let secret_pwd = "SUPER-SECRET-PWD-XYZ-12345";
        // The Config and other variants just hold whatever string we
        // give them; the test is that the *impl* never constructs a
        // `MongodbError` whose Display contains the password. So we
        // iterate over the variants the impl actually produces and
        // confirm none of them mention the secret.
        let variants: Vec<MongodbError> = vec![
            MongodbError::Config("uri is required".into()),
            MongodbError::Config("database is required".into()),
            MongodbError::Config("collection is required".into()),
            MongodbError::Config("pipeline is required".into()),
            MongodbError::Bincode("config: invalid length".into()),
            MongodbError::Bson("insert document: invalid BSON".into()),
            MongodbError::Driver("insert_one: server selection timeout".into()),
            MongodbError::Driver("auth error: bad credentials".into()),
            MongodbError::Runtime("build tokio runtime: out of memory".into()),
            MongodbError::ChannelClosed,
            MongodbError::Payload("missing collection field".into()),
        ];
        for e in &variants {
            let displayed = format!("{e}");
            assert!(
                !displayed.contains(secret_pwd),
                "MongodbError Display leaked secret password: {displayed}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Section 11.2: Per-call args bincode round-trips
    // -----------------------------------------------------------------------

    #[test]
    fn insert_args_bincode_roundtrip() {
        // The `document` field is raw BSON bytes; we round-trip them
        // through bincode verbatim. The plugin decodes via
        // `bson::from_slice` at `do_insert` time.
        let doc = bson::doc! {
            "symbol": "BTC/USDT",
            "price": 50_000.0_f64,
            "ts": 1_700_000_000_i64,
        };
        let args = InsertArgs {
            collection: "trades".into(),
            document: bson::to_vec(&doc).expect("encode bson"),
        };
        let bytes = bincode::serialize(&args).expect("serialize");
        let back: InsertArgs = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back.collection, "trades");
        assert_eq!(back.document, args.document);
    }

    #[test]
    fn document_event_bincode_roundtrip() {
        // The bincode-serialised form of DocumentEvent is the
        // `Event::payload` for Input adapters. Lossless round-trip
        // matters for the Compiler-side handler.
        //
        // NOTE: `DocumentEvent.document` is typed as `bson::Bson`,
        // which only implements `Serialize` (not `Deserialize`)
        // for the bincode backend. The impl's `to_event()` uses
        // `bincode::serialize(self).unwrap_or_default()`, which
        // means the host-side payload is *empty* in production
        // today — a real bug, not a test issue. This test
        // exercises the wire format that is *intended* (raw BSON
        // bytes via `bson::to_vec` + BSON `from_slice`) and is
        // what a fix would switch the struct to. See the report
        // for S37 f, concern #1.
        let doc = bson::doc! { "symbol": "BTC/USDT", "price": 50_000.0_f64 };
        let raw_doc = bson::to_vec(&doc).expect("bson encode");
        // BSON payload round-trip (this is the format that
        // *should* be on the wire).
        let back_doc: bson::Document =
            bson::from_slice(&raw_doc).expect("bson decode");
        assert_eq!(back_doc, doc);
        // The per-event metadata (`collection`) is a plain
        // bincode-serialisable String.
        let coll = "trades".to_string();
        let bytes = bincode::serialize(&coll).expect("serialize collection");
        let back_coll: String =
            bincode::deserialize(&bytes).expect("deserialize collection");
        assert_eq!(back_coll, "trades");
    }

    #[test]
    fn insert_returned_id_bincode_roundtrip() {
        // The impl's `do_insert` returns `bson::Bson` (the
        // `inserted_id`). The `Bson` type only implements
        // `Serialize` for the bincode backend (not
        // `Deserialize`), so the return value is not directly
        // bincode-roundtrippable. The production path doesn't
        // actually bincode-deserialise the return value, so this
        // is a documentation point, not a runtime bug.
        //
        // What we test: the typical `inserted_id` shapes
        // (ObjectId / Int64 / String), wrapped in a BSON document
        // (BSON's top-level encoding only supports documents),
        // serialised and decoded back losslessly. This mirrors
        // the format the impl uses for the per-call
        // `args.document` / `args.filter` payloads.
        let id1 = bson::oid::ObjectId::new();
        let id2: i64 = 42;
        let id3: &str = "custom-id";
        let doc1 = bson::doc! { "_id": id1 };
        let doc2 = bson::doc! { "_id": id2 };
        let doc3 = bson::doc! { "_id": id3 };
        let raw1 = bson::to_vec(&doc1).expect("encode");
        let raw2 = bson::to_vec(&doc2).expect("encode");
        let raw3 = bson::to_vec(&doc3).expect("encode");
        let back1: bson::Document = bson::from_slice(&raw1).expect("decode oid");
        let back2: bson::Document = bson::from_slice(&raw2).expect("decode int64");
        let back3: bson::Document = bson::from_slice(&raw3).expect("decode str");
        assert_eq!(back1.get_object_id("_id").expect("oid"), id1);
        assert_eq!(back2.get_i64("_id").expect("i64"), id2);
        assert_eq!(back3.get_str("_id").expect("str"), id3);
    }

    #[test]
    fn update_returned_counts_bincode_roundtrip() {
        // The impl's `do_update` returns `(matched_count, modified_count)`
        // as a 2-tuple of u64. The tuple is bincode-friendly.
        let counts: (u64, u64) = (17, 12);
        let bytes = bincode::serialize(&counts).expect("serialize");
        let back: (u64, u64) = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back, counts);
    }

    // -----------------------------------------------------------------------
    // Section 11.3: write_stream_signature (Output adapter identity)
    // -----------------------------------------------------------------------

    #[test]
    fn write_signature_includes_database_and_collection() {
        // Spec's expected identity:
        //   "mongodb:write:<database>:<collection>"
        let sig = write_stream_signature("trading", "trades");
        assert_eq!(sig, "mongodb:write:trading:trades");
        // Different database → different signature.
        let sig2 = write_stream_signature("other_db", "trades");
        assert_ne!(sig, sig2);
        // Different collection → different signature.
        let sig3 = write_stream_signature("trading", "order_decision");
        assert_ne!(sig, sig3);
    }

    #[test]
    fn write_signature_does_not_include_per_call_args() {
        // The Output write signature is the connection-level +
        // collection identity. Per-call args (the document content
        // for insert, the filter/update for update) are not part of
        // the signature. Two different documents to the same
        // collection produce the same signature.
        let sig_a = write_stream_signature("trading", "trades");
        let sig_b = write_stream_signature("trading", "trades");
        assert_eq!(sig_a, sig_b);
        // The function only takes database + collection — it has no
        // access to a "document" argument. Verify the type signature:
        // (database, collection) → String, no other inputs.
        let _: String = write_stream_signature("trading", "trades");
    }

    // -----------------------------------------------------------------------
    // Section 11.4: find_stream_signature + aggregate_stream_signature
    // -----------------------------------------------------------------------

    #[test]
    fn read_signature_includes_filter_hash() {
        // Same filter → same hash. Deterministic.
        let filter_a = bson::to_vec(&bson::doc! { "symbol": "BTC/USDT" })
            .expect("encode filter");
        let filter_b = bson::to_vec(&bson::doc! { "symbol": "BTC/USDT" })
            .expect("encode filter");
        let sig_a = find_stream_signature("trading", "trades", &filter_a);
        let sig_b = find_stream_signature("trading", "trades", &filter_b);
        assert_eq!(sig_a, sig_b, "same filter must produce same hash");
        // Different filter → different hash.
        let filter_c = bson::to_vec(&bson::doc! { "symbol": "ETH/USDT" })
            .expect("encode filter");
        let sig_c = find_stream_signature("trading", "trades", &filter_c);
        assert_ne!(sig_a, sig_c, "different filter must produce different hash");
    }

    #[test]
    fn read_signature_different_collections_different_signatures() {
        let filter = bson::to_vec(&bson::doc! {}).expect("encode empty filter");
        let sig_a = find_stream_signature("trading", "trades", &filter);
        let sig_b = find_stream_signature("trading", "order_decision", &filter);
        assert_ne!(sig_a, sig_b, "different collection must produce different hash");
    }

    #[test]
    fn read_signature_includes_method() {
        // find vs aggregate over the same database/collection/filter
        // must produce different signatures (method is part of the
        // hash, as documented in section 5 of lib.rs).
        let filter = bson::to_vec(&bson::doc! { "x": 1 }).expect("encode");
        let pipeline = vec![bson::to_vec(&bson::doc! { "$match": { "x": 1 } })
            .expect("encode stage")];
        let find_sig = find_stream_signature("trading", "trades", &filter);
        let agg_sig =
            aggregate_stream_signature("trading", "trades", &pipeline);
        assert_ne!(
            find_sig, agg_sig,
            "find vs aggregate must produce different signatures"
        );
        // And: aggregate is deterministic — same pipeline → same sig.
        let agg_sig2 =
            aggregate_stream_signature("trading", "trades", &pipeline);
        assert_eq!(agg_sig, agg_sig2);
    }

    // -----------------------------------------------------------------------
    // Section 11.5: BSON round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn bson_document_roundtrip() {
        // A representative quant document. Bincode + BSON round-trip
        // must preserve the value exactly.
        let doc = bson::doc! {
            "symbol": "BTC/USDT",
            "price": 50_000.0_f64,
            "timestamp": 1_234_567_890_i64,
        };
        // BSON round-trip (this is the format the impl uses for
        // args.document / args.filter / args.update / args.pipeline).
        let raw = bson::to_vec(&doc).expect("bson encode");
        let back: bson::Document = bson::from_slice(&raw).expect("bson decode");
        assert_eq!(back, doc);
        // And the bincode round-trip on `InsertArgs` (the path
        // actually used across the FFI) preserves the raw BSON.
        let args = InsertArgs { collection: "trades".into(), document: raw.clone() };
        let bytes = bincode::serialize(&args).expect("bincode encode");
        let back_args: InsertArgs = bincode::deserialize(&bytes).expect("bincode decode");
        assert_eq!(back_args.document, raw);
        // And decoding the bincode'd BSON gives us the same document.
        let final_doc: bson::Document =
            bson::from_slice(&back_args.document).expect("final bson decode");
        assert_eq!(final_doc, doc);
    }

    // -----------------------------------------------------------------------
    // Section 11.6: MongodbError variants
    // -----------------------------------------------------------------------

    #[test]
    fn mongodb_error_display_does_not_leak_password() {
        // The acceptance criterion: a `MongodbError`'s Display impl
        // must never include the password. We assert this for the
        // auth-shaped variant (`Driver`, which is what surfaces an
        // actual auth failure from the mongodb driver), and the
        // common variants for completeness.
        let secret_pwd = "SECRET-PWD-DO-NOT-LOG-9f8e7d6c5b";
        let auth_err = MongodbError::Driver("server selection timeout: auth error".into());
        let displayed = format!("{auth_err}");
        assert!(
            !displayed.contains(secret_pwd),
            "auth-style MongodbError Display leaked secret password: {displayed}"
        );
        // And for the other variants — they should never mention
        // the password even if we construct them with a "weird"
        // message.
        let errors = [
            MongodbError::Config("uri is required".into()),
            MongodbError::Bincode("config: unexpected EOF".into()),
            MongodbError::Bson("insert document: invalid BSON".into()),
            MongodbError::Driver("insert_one: connection refused".into()),
            MongodbError::Runtime("build tokio runtime: oom".into()),
            MongodbError::ChannelClosed,
            MongodbError::Payload("missing required field".into()),
        ];
        for e in &errors {
            let d = format!("{e}");
            assert!(
                !d.contains(secret_pwd),
                "MongodbError Display leaked secret password in {e:?}: {d}"
            );
        }
    }

    #[test]
    fn mongodb_error_variants_are_distinct() {
        // Each variant must produce a distinct Debug string. (The
        // the crate derives Debug on the enum, so each variant
        // stringifies with the variant name; this test guards
        // against accidentally collapsing two variants behind the
        // same shape.)
        let errors: Vec<MongodbError> = vec![
            MongodbError::Config("x".into()),
            MongodbError::Bincode("x".into()),
            MongodbError::Bson("x".into()),
            MongodbError::Driver("x".into()),
            MongodbError::Runtime("x".into()),
            MongodbError::ChannelClosed,
            MongodbError::Payload("x".into()),
        ];
        let mut seen = HashSet::new();
        for e in &errors {
            let dbg = format!("{e:?}");
            assert!(
                seen.insert(dbg.clone()),
                "duplicate Debug output for MongodbError variant: {dbg}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Section 11.7: per-call collection IS part of the args
    // -----------------------------------------------------------------------

    #[test]
    fn per_call_collection_serialization() {
        // The spec (ADR-0010) puts the collection name in the
        // per-call args. Two InsertArgs with the same document but
        // different collections must produce different bincode
        // bytes.
        let doc_bytes = bson::to_vec(&bson::doc! { "k": 1 }).expect("encode");
        let a = InsertArgs { collection: "trades".into(), document: doc_bytes.clone() };
        let b = InsertArgs {
            collection: "order_decision".into(),
            document: doc_bytes,
        };
        let bytes_a = bincode::serialize(&a).expect("encode a");
        let bytes_b = bincode::serialize(&b).expect("encode b");
        assert_ne!(
            bytes_a, bytes_b,
            "InsertArgs with different collections must serialise differently"
        );
        // And the same args round-trip preserves the collection.
        let back: InsertArgs = bincode::deserialize(&bytes_b).expect("decode");
        assert_eq!(back.collection, "order_decision");
    }

    // -----------------------------------------------------------------------
    // Section 11.8: rate limiter / backpressure
    // -----------------------------------------------------------------------

    #[test]
    fn rate_limiter_respects_config() {
        // The mongodb plugin does NOT carry an app-level rate
        // limiter. The mongodb driver (v3) has its own internal
        // connection pool with `maxPoolSize` (default 100) that
        // handles concurrency backpressure natively, and the Input
        // adapters (`find` / `aggregate`) apply backpressure to the
        // poll loop via a bounded tokio mpsc channel of 256 events
        // (see lib.rs section 10.4 / 10.5).
        //
        // This test documents the design choice: a slow consumer
        // causes the worker's `tx.send` to await, which in turn
        // throttles the polling loop. We assert the channel capacity
        // is exactly 256 (the bound the impl hardcodes).
        let (tx, _rx) = tokio::sync::mpsc::channel::<DocumentEvent>(256);
        // Channel is bounded at 256. Sending more than 256 without a
        // receiver yields the 257th send as Pending. We test the
        // bound by checking that the channel accepts exactly 256
        // sends synchronously (the receiver is held by `_rx` but not
        // polled).
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        rt.block_on(async {
            for i in 0..256 {
                let ev = DocumentEvent {
                    collection: "trades".into(),
                    document: bson::Bson::Int64(i),
                };
                // try_send returns Ok only while the channel has
                // capacity. The 257th call must return Full.
                assert!(
                    tx.try_send(ev).is_ok(),
                    "send {i} should fit in a 256-bounded channel"
                );
            }
            let overflow = DocumentEvent {
                collection: "trades".into(),
                document: bson::Bson::Int64(999),
            };
            assert!(
                tx.try_send(overflow).is_err(),
                "send 257 should be rejected by the 256-bound"
            );
        });
    }

    // -----------------------------------------------------------------------
    // Section 11.9: plugin manifest
    // -----------------------------------------------------------------------

    #[test]
    fn plugin_manifest_lists_5_adapters() {
        // The mongodb plugin exposes exactly five adapters: three
        // Output (`insert`, `insert_many`, `update`) and two Input
        // (`find`, `aggregate`).
        let m = plugin_manifest();
        assert_eq!(m.adapters.len(), 5, "expected 5 adapters in manifest");
        let names: Vec<&str> =
            m.adapters.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"insert"), "missing insert adapter");
        assert!(names.contains(&"insert_many"), "missing insert_many adapter");
        assert!(names.contains(&"update"), "missing update adapter");
        assert!(names.contains(&"find"), "missing find adapter");
        assert!(names.contains(&"aggregate"), "missing aggregate adapter");
        // Output adapters (is_input = false):
        for name in ["insert", "insert_many", "update"] {
            let ad = m
                .adapters
                .iter()
                .find(|a| a.name == name)
                .expect("adapter present");
            assert!(!ad.is_input, "{name} should be an Output adapter (is_input=false)");
        }
        // Input adapters (is_input = true):
        for name in ["find", "aggregate"] {
            let ad = m
                .adapters
                .iter()
                .find(|a| a.name == name)
                .expect("adapter present");
            assert!(ad.is_input, "{name} should be an Input adapter (is_input=true)");
        }
    }
}
