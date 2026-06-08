//! `bee-plugin-mongodb-mock` — S33 mock plugin.
//!
//! Implements the `mongodb_emit` Output Adapter. Writes each emitted
//! Event as a single JSON object on its own line (JSONL) to a local
//! file — default `/tmp/bee_demo_mongodb.jsonl`. The file is opened
//! in append mode so multiple emitters and restarts accumulate rows
//! in chronological order.
//!
//! ## Architecture
//!
//! - [`MongodbMockFactory`]: produces the
//!   [`bee_plugin_sdk::PluginManifest`] + [`bee_plugin_sdk::PluginHandle`]
//!   for the host.
//! - [`MongodbMockOutput`]: the actual [`bee_adapter::OutputAdapter`]
//!   implementation. Wraps a `tokio::fs::File` in a
//!   `tokio::io::BufWriter` for batched writes and flushes on close.
//! - `cdylib_plugin!(Factory)` invocation at the bottom generates
//!   the FFI entry symbols.
//!
//! The mock writes **structured** records (not raw payloads):
//! every line is a JSON object with the destination
//! (`_database`, `_collection`), the event envelope
//! (`timestamp`, `sequence`), and the decoded payload string. That
//! matches what a downstream `order_decision` consumer would expect
//! when replaying the demo file.

use std::path::PathBuf;

use bee_adapter::{AdapterError, AdapterResult, Event, OutputAdapter};
use bee_plugin_sdk::{
    vtable::OutputAdapterVtable, AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName,
};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncWriteExt, BufWriter};

/// Default sink path used when the config's `path` field is `None`.
pub const DEFAULT_PATH: &str = "/tmp/bee_demo_mongodb.jsonl";

/// Configuration for the mock mongodb output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MongodbMockConfig {
    /// Target database name. Embedded in every emitted line as
    /// `_database` for downstream routing.
    pub database: String,
    /// Target collection name. Embedded in every emitted line as
    /// `_collection`.
    pub collection: String,
    /// Sink file path. `None` means [`DEFAULT_PATH`].
    pub path: Option<PathBuf>,
}

impl Default for MongodbMockConfig {
    fn default() -> Self {
        Self {
            database: "trading".into(),
            collection: "order_decision".into(),
            path: None,
        }
    }
}

impl MongodbMockConfig {
    /// Resolve the configured path or fall back to [`DEFAULT_PATH`].
    pub fn resolved_path(&self) -> PathBuf {
        self.path
            .clone()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PATH))
    }
}

/// Mock `mongodb_emit` Output Adapter. Appends one JSON object per
/// event to a local file.
pub struct MongodbMockOutput {
    config: MongodbMockConfig,
    writer: BufWriter<tokio::fs::File>,
}

impl OutputAdapter for MongodbMockOutput {
    type Config = MongodbMockConfig;

    async fn open(config: Self::Config) -> AdapterResult<Self> {
        let path = config.resolved_path();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| AdapterError::Open(format!("{}: {e}", path.display())))?;
        Ok(Self {
            config,
            writer: BufWriter::new(file),
        })
    }

    async fn emit(&mut self, event: Event) -> AdapterResult<()> {
        let payload = String::from_utf8_lossy(&event.payload).into_owned();
        let line = serde_json::json!({
            "_database": self.config.database,
            "_collection": self.config.collection,
            "timestamp": event.timestamp,
            "sequence": event.sequence,
            "payload": payload,
        })
        .to_string();
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(|e| AdapterError::Emit(e.to_string()))?;
        self.writer
            .write_all(b"\n")
            .await
            .map_err(|e| AdapterError::Emit(e.to_string()))?;
        Ok(())
    }

    async fn close(self) -> AdapterResult<()> {
        let mut writer = self.writer;
        writer
            .flush()
            .await
            .map_err(|e| AdapterError::Close(e.to_string()))?;
        Ok(())
    }
}

/// Factory for the mongodb mock plugin. Holds no state; both
/// methods are pure.
pub struct MongodbMockFactory;

impl Factory for MongodbMockFactory {
    fn manifest() -> PluginManifest {
        PluginManifest {
            name: PluginName("mongodb".into()),
            feature_version: "1.0.0".into(),
            abi_version: "v1".into(),
            adapters: vec![AdapterDescriptor {
                name: "emit".into(),
                is_input: false,
            }],
            handlers: vec![],
        }
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        let vtable: *const OutputAdapterVtable = &vtable_shim::VTABLE;
        let mut output_adapters = std::collections::HashMap::new();
        output_adapters.insert("emit".to_string(), vtable);
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
            input_adapters: std::collections::HashMap::new(),
            output_adapters,
            handlers: std::collections::HashMap::new(),
        })
    }
}

mod vtable_shim {
    use std::sync::Mutex;

    use bee_adapter::OutputAdapter;
    use bee_plugin_sdk::event::{decode_event, EventBytes};
    use bee_plugin_sdk::vtable::OutputAdapterVtable;

    use super::{MongodbMockConfig, MongodbMockOutput};

    pub struct Ctx {
        pub adapter: Mutex<MongodbMockOutput>,
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
        let cfg: MongodbMockConfig = match bincode::deserialize(bytes) {
            Ok(c) => c,
            Err(_) => return std::ptr::null_mut(),
        };
        let adapter = match block_on(MongodbMockOutput::open(cfg)) {
            Ok(a) => a,
            Err(_) => return std::ptr::null_mut(),
        };
        let ctx = Box::new(Ctx {
            adapter: Mutex::new(adapter),
        });
        Box::into_raw(ctx) as *mut std::ffi::c_void
    }

    pub unsafe extern "C" fn emit(
        ctx: *mut std::ffi::c_void,
        event_ptr: *const u8,
        event_len: usize,
    ) -> i32 {
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let event = match decode_event(event_bytes) {
            Ok(e) => e,
            Err(_) => return -1,
        };
        let result = block_on(async move {
            let ctx = &*(ctx as *const Ctx);
            let mut adapter = ctx.adapter.lock().unwrap();
            adapter.emit(event).await
        });
        match result {
            Ok(()) => 0,
            Err(_) => -1,
        }
    }

    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if ctx.is_null() {
            return 0;
        }
        let ctx = Box::from_raw(ctx as *mut Ctx);
        let adapter = ctx.adapter.into_inner().expect("mutex poisoned");
        match block_on(adapter.close()) {
            Ok(()) => 0,
            Err(_) => -1,
        }
    }

    pub const VTABLE: OutputAdapterVtable = OutputAdapterVtable {
        open,
        emit,
        close,
    };
}

bee_plugin_sdk::cdylib_plugin!(MongodbMockFactory);

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Build a unique per-test path under `/tmp` so tests don't
    /// collide with each other or with real demo runs.
    fn test_path(name: &str) -> PathBuf {
        PathBuf::from(format!("/tmp/bee_test_mongodb_{name}.jsonl"))
    }

    /// Remove the test file if it exists. Called before each
    /// file-writing test so the run is hermetic.
    async fn clean(path: &Path) {
        let _ = tokio::fs::remove_file(path).await;
    }

    fn make_event(seq: u64, payload: &[u8]) -> Event {
        Event {
            timestamp: 1_000_000 + seq * 1000,
            sequence: seq,
            payload: payload.to_vec(),
        }
    }

    #[tokio::test]
    async fn emit_writes_jsonl_per_event() {
        let path = test_path("emit_writes_jsonl_per_event");
        clean(&path).await;
        let cfg = MongodbMockConfig {
            path: Some(path.clone()),
            ..Default::default()
        };
        let mut out = MongodbMockOutput::open(cfg).await.unwrap();
        for i in 0..3 {
            out.emit(make_event(i, format!("row-{i}").as_bytes()))
                .await
                .unwrap();
        }
        out.close().await.unwrap();

        let body = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 3);

        for (i, line) in lines.iter().enumerate() {
            let v: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("line {i} not valid JSON: {e}: {line}"));
            assert_eq!(v["_database"], "trading");
            assert_eq!(v["_collection"], "order_decision");
            assert_eq!(v["timestamp"], 1_000_000 + (i as u64) * 1000);
            assert_eq!(v["sequence"], i as u64);
            assert_eq!(v["payload"], format!("row-{i}"));
        }
    }

    #[tokio::test]
    async fn payload_is_preserved_as_string() {
        let path = test_path("payload_is_preserved_as_string");
        clean(&path).await;
        let cfg = MongodbMockConfig {
            path: Some(path.clone()),
            ..Default::default()
        };
        let mut out = MongodbMockOutput::open(cfg).await.unwrap();
        out.emit(make_event(0, b"hello world payload"))
            .await
            .unwrap();
        out.close().await.unwrap();

        let body = tokio::fs::read_to_string(&path).await.unwrap();
        let v: serde_json::Value =
            serde_json::from_str(body.trim()).expect("file is not a JSON object");
        assert_eq!(v["payload"], "hello world payload");
    }

    #[test]
    fn default_path_is_tmp_bee_demo() {
        let cfg = MongodbMockConfig::default();
        assert_eq!(cfg.resolved_path(), PathBuf::from(DEFAULT_PATH));
        assert_eq!(DEFAULT_PATH, "/tmp/bee_demo_mongodb.jsonl");
    }

    #[test]
    fn factory_manifest_declares_emit_adapter() {
        let m = MongodbMockFactory::manifest();
        assert_eq!(m.name.0, "mongodb");
        assert_eq!(m.feature_version, "1.0.0");
        assert_eq!(m.abi_version, "v1");
        assert_eq!(m.adapters.len(), 1);
        assert_eq!(m.adapters[0].name, "emit");
        assert!(!m.adapters[0].is_input);
    }

    #[test]
    fn factory_init_returns_handle_with_manifest() {
        let h = MongodbMockFactory::init().unwrap();
        assert_eq!(h.manifest.name.0, "mongodb");
        assert_eq!(h.manifest.adapters.len(), 1);
        assert_eq!(h.manifest.adapters[0].name, "emit");
        assert!(!h.manifest.adapters[0].is_input);
    }

    #[test]
    fn vtable_open_emit_close_writes_jsonl() {
        let path = test_path("vtable_open_emit_close_writes_jsonl");
        let _ = std::fs::remove_file(&path);
        let cfg = MongodbMockConfig {
            path: Some(path.clone()),
            ..Default::default()
        };
        let handle = MongodbMockFactory::init().expect("init");
        let vtable = *handle
            .output_adapters
            .get("emit")
            .expect("emit vtable");
        let cfg_bytes = bincode::serialize(&cfg).unwrap();
        let ctx = unsafe {
            ((*vtable).open)(cfg_bytes.as_ptr(), cfg_bytes.len(), std::ptr::null_mut())
        };
        assert!(!ctx.is_null(), "open returned null");
        let event = Event {
            timestamp: 2_000_000,
            sequence: 7,
            payload: b"vtable-row".to_vec(),
        };
        let event_bytes = bincode::serialize(&event).unwrap();
        let rc = unsafe {
            ((*vtable).emit)(
                ctx,
                event_bytes.as_ptr(),
                event_bytes.len(),
            )
        };
        assert_eq!(rc, 0, "emit returned {rc}");
        let rc = unsafe { ((*vtable).close)(ctx) };
        assert_eq!(rc, 0, "close returned {rc}");
        let body = std::fs::read_to_string(&path).expect("read jsonl");
        let v: serde_json::Value =
            serde_json::from_str(body.trim()).expect("file is not JSON");
        assert_eq!(v["sequence"], 7);
        assert_eq!(v["payload"], "vtable-row");
        assert_eq!(v["_database"], "trading");
        assert_eq!(v["_collection"], "order_decision");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn vtable_open_with_garbage_config_returns_null() {
        let handle = MongodbMockFactory::init().expect("init");
        let vtable = *handle
            .output_adapters
            .get("emit")
            .expect("emit vtable");
        let garbage = vec![0xFFu8; 8];
        let ctx = unsafe {
            ((*vtable).open)(garbage.as_ptr(), garbage.len(), std::ptr::null_mut())
        };
        assert!(ctx.is_null(), "open with garbage should return null");
    }
}
