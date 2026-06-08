//! `bee-plugin-influxdb-mock` — S33 mock plugin.
//!
//! Implements the `influxdb_emit` Output Adapter. Writes each
//! event to a local append-only log file (default
//! `/tmp/bee_demo_influxdb.log`) in line-protocol-ish text so a
//! downstream MACD / EMA Pipeline can be observed end-to-end
//! without an actual InfluxDB instance.
//!
//! ## Architecture
//!
//! - [`Factory`]: produces the [`bee_plugin_sdk::PluginManifest`]
//!   + [`bee_plugin_sdk::PluginHandle`] for the host.
//! - [`InfluxMockOutput`]: the actual [`bee_adapter::OutputAdapter`]
//!   implementation. `open` creates/opens the log file in append
//!   mode; `emit` appends one line per event; `close` flushes the
//!   buffered writer and drops the file handle.
//! - `cdylib_plugin!(Factory)` invocation at the bottom generates
//!   the FFI entry symbols.
//!
//! The mock is a **sink**: events go in, lines on disk come out.
//! No background task — `emit` is called by the host's dataflow
//! runtime for each event that reaches the Phase.

use std::path::PathBuf;

use bee_adapter::{AdapterError, AdapterResult, Event, OutputAdapter};
use bee_plugin_sdk::{
    vtable::OutputAdapterVtable, AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName,
};
use tokio::io::{AsyncWriteExt, BufWriter};

/// Configuration for the mock influxdb output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InfluxMockConfig {
    /// Logical database name. Goes into the line-protocol-ish
    /// header as a tag, e.g. `database=bitcoin`.
    pub database: String,
    /// Measurement (table) name. Goes into the line-protocol-ish
    /// header as the measurement, e.g. `trade`.
    pub measurement: String,
    /// Output file path. `None` = [`Self::DEFAULT_PATH`].
    pub path: Option<PathBuf>,
}

impl InfluxMockConfig {
    /// Default log file path used when `path` is `None`.
    pub const DEFAULT_PATH: &str = "/tmp/bee_demo_influxdb.log";

    /// Resolve the effective output path: `self.path` if `Some`,
    /// otherwise [`Self::DEFAULT_PATH`].
    pub fn resolved_path(&self) -> PathBuf {
        self.path
            .clone()
            .unwrap_or_else(|| PathBuf::from(Self::DEFAULT_PATH))
    }
}

impl Default for InfluxMockConfig {
    fn default() -> Self {
        Self {
            database: "bitcoin".into(),
            measurement: "trade".into(),
            path: None,
        }
    }
}

/// Mock `influxdb_emit` Output Adapter. Appends one line per
/// event to the configured log file in line-protocol-ish format:
///
/// ```text
/// <measurement>,database=<database> sequence=<seq>,timestamp=<ts> value="<payload>"
/// ```
pub struct InfluxMockOutput {
    config: InfluxMockConfig,
    writer: BufWriter<tokio::fs::File>,
}

impl OutputAdapter for InfluxMockOutput {
    type Config = InfluxMockConfig;

    async fn open(config: Self::Config) -> AdapterResult<Self> {
        let path = config.resolved_path();
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| {
                AdapterError::Open(format!(
                    "open log file {}: {e}",
                    path.display()
                ))
            })?;
        Ok(Self {
            config,
            writer: BufWriter::new(file),
        })
    }

    async fn emit(&mut self, event: Event) -> AdapterResult<()> {
        let payload = String::from_utf8_lossy(&event.payload);
        let line = format!(
            "{measurement},database={database} sequence={sequence},timestamp={timestamp} value=\"{payload}\"\n",
            measurement = self.config.measurement,
            database = self.config.database,
            sequence = event.sequence,
            timestamp = event.timestamp,
            payload = payload,
        );
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(|e| AdapterError::Emit(format!("write line: {e}")))?;
        Ok(())
    }

    async fn close(mut self) -> AdapterResult<()> {
        // Explicit flush — tokio's `BufWriter` does NOT flush on
        // drop (to avoid blocking in async contexts), so the
        // buffered bytes would be discarded without this.
        self.writer
            .flush()
            .await
            .map_err(|e| AdapterError::Close(format!("flush: {e}")))?;
        // File handle is dropped at end of function, releasing
        // the OS-level fd.
        Ok(())
    }
}

/// Factory for the influxdb mock plugin. Holds no state; both
/// methods are pure.
pub struct InfluxMockFactory;

impl Factory for InfluxMockFactory {
    fn manifest() -> PluginManifest {
        PluginManifest {
            name: PluginName("influxdb".into()),
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

    use super::{InfluxMockConfig, InfluxMockOutput};

    pub struct Ctx {
        pub adapter: Mutex<InfluxMockOutput>,
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
        let cfg: InfluxMockConfig = match bincode::deserialize(bytes) {
            Ok(c) => c,
            Err(_) => return std::ptr::null_mut(),
        };
        let adapter = match block_on(InfluxMockOutput::open(cfg)) {
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

bee_plugin_sdk::cdylib_plugin!(InfluxMockFactory);

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Local helper: build a tempdir-style path unique to this
    /// test invocation. Avoids polluting
    /// `/tmp/bee_demo_influxdb.log` between runs.
    fn unique_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        PathBuf::from(format!(
            "/tmp/bee_test_influxdb_{name}_{pid}_{nanos}.log",
            name = name,
            pid = std::process::id(),
            nanos = nanos,
        ))
    }

    /// Build a synthetic Event for emit-side tests.
    fn make_event(seq: u64, payload: &[u8]) -> Event {
        Event {
            timestamp: 1_000_000 + seq * 1000,
            sequence: seq,
            payload: payload.to_vec(),
        }
    }

    #[tokio::test]
    async fn emit_writes_line_per_event() {
        let path = unique_path("emit_writes_line_per_event");
        let cfg = InfluxMockConfig {
            path: Some(path.clone()),
            ..Default::default()
        };
        let mut adapter = InfluxMockOutput::open(cfg).await.unwrap();
        for i in 0..3 {
            adapter.emit(make_event(i, b"hello")).await.unwrap();
        }
        adapter.close().await.unwrap();

        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 lines, got: {contents}");
        // First line has sequence=0; last line has sequence=2.
        assert!(lines[0].contains("sequence=0"), "line 0: {}", lines[0]);
        assert!(lines[2].contains("sequence=2"), "line 2: {}", lines[2]);
        // Header / payload shape.
        for (i, line) in lines.iter().enumerate() {
            assert!(
                line.starts_with("trade,database=bitcoin "),
                "line {i}: {line}"
            );
            assert!(line.contains("value=\"hello\""), "line {i}: {line}");
        }
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[test]
    fn default_path_is_tmp_bee_demo() {
        assert_eq!(
            InfluxMockConfig::default().resolved_path(),
            PathBuf::from("/tmp/bee_demo_influxdb.log"),
        );
    }

    #[tokio::test]
    async fn close_flushes_buffer() {
        let path = unique_path("close_flushes_buffer");
        let cfg = InfluxMockConfig {
            path: Some(path.clone()),
            ..Default::default()
        };
        let mut adapter = InfluxMockOutput::open(cfg).await.unwrap();
        adapter.emit(make_event(0, b"flushed")).await.unwrap();
        // Before close: BufWriter has the line buffered. tokio's
        // `BufWriter` does NOT flush on drop (to avoid blocking),
        // so the data would be lost if `close` did not flush.
        adapter.close().await.unwrap();
        // After close: the buffered data must be on disk.
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            contents.contains("flushed"),
            "missing payload after close: {contents}"
        );
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[test]
    fn factory_manifest_declares_emit_adapter() {
        let m = InfluxMockFactory::manifest();
        assert_eq!(m.name.0, "influxdb");
        assert_eq!(m.abi_version, "v1");
        assert_eq!(m.adapters.len(), 1);
        assert_eq!(m.adapters[0].name, "emit");
        assert!(!m.adapters[0].is_input);
    }

    #[test]
    fn factory_init_returns_handle_with_manifest() {
        let h = InfluxMockFactory::init().unwrap();
        assert_eq!(h.manifest.name.0, "influxdb");
        assert_eq!(h.manifest.adapters.len(), 1);
        assert_eq!(h.manifest.adapters[0].name, "emit");
    }

    #[test]
    fn vtable_open_emit_close_writes_event_to_log() {
        let path = unique_path("vtable_open_emit_close");
        let cfg = InfluxMockConfig {
            path: Some(path.clone()),
            ..Default::default()
        };
        let handle = InfluxMockFactory::init().expect("init");
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
            timestamp: 1_000_000,
            sequence: 42,
            payload: b"vtable-payload".to_vec(),
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
        let body = std::fs::read_to_string(&path).expect("read log");
        assert!(body.contains("sequence=42"), "missing sequence: {body}");
        assert!(body.contains("vtable-payload"), "missing payload: {body}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn vtable_open_with_garbage_config_returns_null() {
        let handle = InfluxMockFactory::init().expect("init");
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
