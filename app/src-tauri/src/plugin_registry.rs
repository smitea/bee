use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use bee_plugin_sdk::{AdapterDescriptor, HandlerDescriptor, PluginHandle, PluginManifest};
use libloading::{Library, Symbol};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum PluginRegistryError {
    #[error("dlopen {path}: {source}")]
    Dlopen {
        path: String,
        #[source]
        source: libloading::Error,
    },
    #[error("read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("resolve `bee_plugin_init` in {path}: {source}")]
    ResolveInit {
        path: String,
        #[source]
        source: libloading::Error,
    },
    #[error("`bee_plugin_init` returned null in {path}")]
    InitReturnedNull { path: String },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub adapters: Vec<String>,
    pub handlers: Vec<String>,
}

pub struct LoadedEntry {
    pub id: String,
    pub manifest: PluginManifest,
    #[allow(dead_code)]
    pub lib: Option<Library>,
    #[allow(dead_code)]
    pub handle: *mut PluginHandle,
}

unsafe impl Send for LoadedEntry {}
unsafe impl Sync for LoadedEntry {}

impl Clone for LoadedEntry {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            manifest: self.manifest.clone(),
            lib: None,
            handle: self.handle,
        }
    }
}

pub struct PluginRegistry {
    inner: Mutex<HashMap<String, LoadedEntry>>,
}

unsafe impl Send for PluginRegistry {}
unsafe impl Sync for PluginRegistry {}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn scan_directory(&self, path: &Path) -> Vec<PluginSummary> {
        let mut loaded_ids: Vec<String> = Vec::new();
        let ext = std::env::consts::DLL_EXTENSION;
        let read = match std::fs::read_dir(path) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        for entry in read.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let matches = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.eq_ignore_ascii_case(ext))
                .unwrap_or(false);
            if !matches {
                continue;
            }
            if let Ok(id) = self.load(&p) {
                loaded_ids.push(id);
            }
        }
        let mut summaries = self.list_summaries();
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        summaries.retain(|s| loaded_ids.iter().any(|id| id == &s.id));
        summaries
    }

    pub fn load(&self, path: &Path) -> Result<String, PluginRegistryError> {
        let path_str = path.display().to_string();
        let bytes = std::fs::read(path).map_err(|e| PluginRegistryError::Read {
            path: path_str.clone(),
            source: e,
        })?;
        let id = compute_id(&bytes);
        let (manifest, lib, handle) = unsafe { load_entry(path, &path_str)? };
        let mut guard = self.inner.lock().unwrap();
        guard.insert(
            id.clone(),
            LoadedEntry {
                id: id.clone(),
                manifest,
                lib: Some(lib),
                handle,
            },
        );
        Ok(id)
    }

    pub fn loaded_plugins(&self) -> Vec<(String, PluginManifest)> {
        let guard = self.inner.lock().unwrap();
        let mut out: Vec<(String, PluginManifest)> = guard
            .values()
            .map(|e| (e.id.clone(), e.manifest.clone()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn list_summaries(&self) -> Vec<PluginSummary> {
        let entries: Vec<LoadedEntry> = {
            let guard = self.inner.lock().unwrap();
            guard.values().cloned().collect()
        };
        summaries_from_entries(&entries)
    }

    pub fn manifest(&self, id: &str) -> Option<PluginManifest> {
        let guard = self.inner.lock().unwrap();
        guard.get(id).map(|e| e.manifest.clone())
    }

    pub fn insert_manifest(&self, id: String, manifest: PluginManifest) {
        let mut guard = self.inner.lock().unwrap();
        guard.insert(
            id.clone(),
            LoadedEntry {
                id,
                manifest,
                lib: None,
                handle: std::ptr::null_mut(),
            },
        );
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn summaries_from_entries(entries: &[LoadedEntry]) -> Vec<PluginSummary> {
    let mut sorted: Vec<&LoadedEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    sorted
        .into_iter()
        .map(|e| PluginSummary {
            id: e.id.clone(),
            name: e.manifest.name.0.clone(),
            version: e.manifest.feature_version.clone(),
            adapters: e
                .manifest
                .adapters
                .iter()
                .map(|a: &AdapterDescriptor| a.name.clone())
                .collect(),
            handlers: e
                .manifest
                .handlers
                .iter()
                .map(|h: &HandlerDescriptor| h.name.clone())
                .collect(),
        })
        .collect()
}

fn compute_id(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

unsafe fn load_entry(
    path: &Path,
    path_str: &str,
) -> Result<(PluginManifest, Library, *mut PluginHandle), PluginRegistryError> {
    let lib = Library::new(path).map_err(|e| PluginRegistryError::Dlopen {
        path: path_str.to_string(),
        source: e,
    })?;
    let init: Symbol<unsafe extern "C" fn(*mut bee_plugin_sdk::BeeHostV1) -> *mut PluginHandle> =
        lib.get(b"bee_plugin_init").map_err(|e| PluginRegistryError::ResolveInit {
            path: path_str.to_string(),
            source: e,
        })?;
    let mut host = Box::new(bee_plugin_sdk::BeeHostV1 {
        ctx: std::ptr::null_mut(),
        register_adapter: None,
        register_input_adapter_vtable: None,
        register_output_adapter_vtable: None,
        register_handler_vtable: None,
        kv_get: None,
        kv_put: None,
        kv_cas: None,
        secret_get: None,
        secret_put: None,
        current_stream_id: None,
    });
    let host_ptr: *mut bee_plugin_sdk::BeeHostV1 = &mut *host;
    let handle_ptr = init(host_ptr);
    if handle_ptr.is_null() {
        return Err(PluginRegistryError::InitReturnedNull {
            path: path_str.to_string(),
        });
    }
    let manifest = (*handle_ptr).manifest.clone();
    Ok((manifest, lib, handle_ptr))
}

pub fn schema_for(manifest: &PluginManifest) -> serde_json::Value {
    let mut adapters = serde_json::Map::new();
    for a in &manifest.adapters {
        let mut connection = serde_json::Map::new();
        let mut url = serde_json::Map::new();
        url.insert("type".into(), serde_json::Value::String("string".into()));
        url.insert("required".into(), serde_json::Value::Bool(true));
        connection.insert("url".into(), serde_json::Value::Object(url));

        let mut credentials = serde_json::Map::new();
        credentials.insert("type".into(), serde_json::Value::String("object".into()));
        credentials.insert("required".into(), serde_json::Value::Bool(false));
        let cred_props = serde_json::Map::new();
        credentials.insert("properties".into(), serde_json::Value::Object(cred_props));
        connection.insert("credentials".into(), serde_json::Value::Object(credentials));

        let mut rate_limit = serde_json::Map::new();
        rate_limit.insert("type".into(), serde_json::Value::String("integer".into()));
        rate_limit.insert("required".into(), serde_json::Value::Bool(false));
        connection.insert("rate_limit".into(), serde_json::Value::Object(rate_limit));

        let mut adapter_entry = serde_json::Map::new();
        adapter_entry.insert(
            "type".into(),
            serde_json::Value::String(if a.is_input { "input" } else { "output" }.into()),
        );
        adapter_entry.insert(
            "connection".into(),
            serde_json::Value::Object(connection),
        );
        adapters.insert(a.name.clone(), serde_json::Value::Object(adapter_entry));
    }
    serde_json::Value::Object(adapters)
}

pub fn placeholder_schema(name: &str) -> serde_json::Value {
    let mut connection = serde_json::Map::new();
    let mut url = serde_json::Map::new();
    url.insert("type".into(), serde_json::Value::String("string".into()));
    url.insert("required".into(), serde_json::Value::Bool(true));
    connection.insert("url".into(), serde_json::Value::Object(url));
    let mut credentials = serde_json::Map::new();
    credentials.insert("type".into(), serde_json::Value::String("object".into()));
    credentials.insert("required".into(), serde_json::Value::Bool(false));
    connection.insert("credentials".into(), serde_json::Value::Object(credentials));
    let mut rate_limit = serde_json::Map::new();
    rate_limit.insert("type".into(), serde_json::Value::String("integer".into()));
    rate_limit.insert("required".into(), serde_json::Value::Bool(false));
    connection.insert("rate_limit".into(), serde_json::Value::Object(rate_limit));
    let mut adapter_entry = serde_json::Map::new();
    adapter_entry.insert("type".into(), serde_json::Value::String("input".into()));
    adapter_entry.insert("connection".into(), serde_json::Value::Object(connection));
    let mut adapters = serde_json::Map::new();
    adapters.insert(name.to_string(), serde_json::Value::Object(adapter_entry));
    serde_json::Value::Object(adapters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_plugin_sdk::PluginName;

    #[test]
    fn empty_registry_returns_empty_list() {
        let reg = PluginRegistry::new();
        assert!(reg.list_summaries().is_empty());
        assert!(reg.loaded_plugins().is_empty());
    }

    #[test]
    fn load_missing_file_returns_error() {
        let reg = PluginRegistry::new();
        let result = reg.load(Path::new("/tmp/this_does_not_exist_bee_plugin_xyz.dylib"));
        assert!(result.is_err());
    }

    #[test]
    fn compute_id_is_deterministic_sha256_hex() {
        assert_eq!(compute_id(b"hello").len(), 64);
        assert_eq!(compute_id(b"hello"), compute_id(b"hello"));
        assert_ne!(compute_id(b"hello"), compute_id(b"world"));
    }

    #[test]
    fn summaries_from_entries_sorts_by_id_and_extracts_fields() {
        let entries = vec![
            LoadedEntry {
                id: "z-id".into(),
                manifest: PluginManifest {
                    name: PluginName("z".into()),
                    feature_version: "2.0.0".into(),
                    abi_version: "v1".into(),
                    adapters: vec![AdapterDescriptor {
                        name: "out".into(),
                        is_input: false,
                    }],
                    handlers: vec![HandlerDescriptor {
                        name: "do_thing".into(),
                    }],
                },
                lib: None,
                handle: std::ptr::null_mut(),
            },
            LoadedEntry {
                id: "a-id".into(),
                manifest: PluginManifest {
                    name: PluginName("a".into()),
                    feature_version: "1.2.3".into(),
                    abi_version: "v1".into(),
                    adapters: vec![AdapterDescriptor {
                        name: "subscribe".into(),
                        is_input: true,
                    }],
                    handlers: vec![],
                },
                lib: None,
                handle: std::ptr::null_mut(),
            },
        ];
        let summaries = summaries_from_entries(&entries);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].id, "a-id");
        assert_eq!(summaries[1].id, "z-id");
        assert_eq!(summaries[0].name, "a");
        assert_eq!(summaries[0].version, "1.2.3");
        assert_eq!(summaries[0].adapters, vec!["subscribe".to_string()]);
        assert!(summaries[0].handlers.is_empty());
        assert_eq!(summaries[1].adapters, vec!["out".to_string()]);
        assert_eq!(summaries[1].handlers, vec!["do_thing".to_string()]);
    }

    #[test]
    fn insert_manifest_then_list_returns_one_summary() {
        let reg = PluginRegistry::new();
        reg.insert_manifest(
            "abc".into(),
            PluginManifest {
                name: PluginName("test".into()),
                feature_version: "1.2.3".into(),
                abi_version: "v1".into(),
                adapters: vec![AdapterDescriptor {
                    name: "subscribe".into(),
                    is_input: true,
                }],
                handlers: vec![HandlerDescriptor {
                    name: "fib".into(),
                }],
            },
        );
        let summaries = reg.list_summaries();
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.id, "abc");
        assert_eq!(s.name, "test");
        assert_eq!(s.version, "1.2.3");
        assert_eq!(s.adapters, vec!["subscribe".to_string()]);
        assert_eq!(s.handlers, vec!["fib".to_string()]);
    }

    #[test]
    fn insert_manifest_then_manifest_lookup_returns_clone() {
        let reg = PluginRegistry::new();
        let original = PluginManifest {
            name: PluginName("p".into()),
            feature_version: "1.0.0".into(),
            abi_version: "v1".into(),
            adapters: vec![],
            handlers: vec![],
        };
        reg.insert_manifest("id1".into(), original.clone());
        assert_eq!(reg.manifest("id1"), Some(original));
        assert!(reg.manifest("missing").is_none());
    }

    #[test]
    fn schema_for_manifest_has_connection_with_url_credentials_rate_limit() {
        let m = PluginManifest {
            name: PluginName("p".into()),
            feature_version: "1.0.0".into(),
            abi_version: "v1".into(),
            adapters: vec![AdapterDescriptor {
                name: "subscribe".into(),
                is_input: true,
            }],
            handlers: vec![],
        };
        let schema = schema_for(&m);
        let adapter = schema.get("subscribe").expect("adapter key");
        let connection = adapter.get("connection").expect("connection key");
        assert!(connection.get("url").is_some());
        assert!(connection.get("credentials").is_some());
        assert!(connection.get("rate_limit").is_some());
        assert_eq!(
            connection.get("url").unwrap().get("type").unwrap(),
            &serde_json::Value::String("string".into())
        );
        assert_eq!(
            connection.get("rate_limit").unwrap().get("type").unwrap(),
            &serde_json::Value::String("integer".into())
        );
    }

    #[test]
    fn placeholder_schema_returns_same_shape_for_unknown_name() {
        let s = placeholder_schema("binance");
        let adapter = s.get("binance").unwrap();
        let connection = adapter.get("connection").unwrap();
        assert!(connection.get("url").is_some());
        assert!(connection.get("credentials").is_some());
        assert!(connection.get("rate_limit").is_some());
    }

    #[test]
    fn load_real_compiled_cdylib_returns_summary_with_id_and_manifest() {
        let Some(cdylib_path) = build_named_test_cdylib("bee-plugin-test-fixture", None) else {
            eprintln!("rustc not on PATH or compile failed; skipping cdylib load test");
            return;
        };
        let reg = PluginRegistry::new();
        let id = reg.load(&cdylib_path).expect("load cdylib");
        assert_eq!(id.len(), 64, "id should be sha256 hex");
        let summaries = reg.list_summaries();
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.id, id);
        assert_eq!(s.name, "bee-plugin-test-fixture");
        assert_eq!(s.adapters, vec!["subscribe".to_string()]);
        assert!(s.handlers.is_empty());
    }

    #[test]
    fn scan_directory_returns_plugins_sorted_by_name() {
        let dir = tempdir_in_target();
        let Some(_zeta) = build_named_test_cdylib("zeta-plugin", Some(&dir)) else {
            eprintln!("rustc not on PATH or compile failed; skipping scan_directory test");
            return;
        };
        let Some(_alpha) = build_named_test_cdylib("alpha-plugin", Some(&dir)) else {
            eprintln!("rustc not on PATH or compile failed; skipping scan_directory test");
            return;
        };
        let reg = PluginRegistry::new();
        let summaries = reg.scan_directory(&dir);
        assert_eq!(summaries.len(), 2, "expected two plugins loaded");
        assert_eq!(summaries[0].name, "alpha-plugin");
        assert_eq!(summaries[1].name, "zeta-plugin");
    }

    #[test]
    fn scan_directory_nonexistent_returns_empty_list() {
        let reg = PluginRegistry::new();
        let summaries = reg.scan_directory(Path::new("/tmp/bee_plugin_scan_no_such_dir_xyz"));
        assert!(summaries.is_empty());
    }

    #[test]
    fn scan_directory_skips_non_plugin_files() {
        let dir = tempdir_in_target();
        std::fs::write(dir.join("readme.txt"), b"not a plugin").unwrap();
        std::fs::write(dir.join("config.json"), b"{}").unwrap();
        let reg = PluginRegistry::new();
        let summaries = reg.scan_directory(&dir);
        assert!(summaries.is_empty());
    }
}

#[cfg(test)]
fn build_named_test_cdylib(
    plugin_name: &str,
    out_dir: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let target = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target");
    let target = if target.is_dir() { target } else { std::env::current_dir().ok()?.join("target") };
    let deps = target.join("debug").join("deps");
    if !deps.is_dir() {
        return None;
    }
    let sdk_rlib = std::fs::read_dir(&deps)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with("libbee_plugin_sdk-") && s.ends_with(".rlib"))
                .unwrap_or(false)
        })?;
    let build_dir = match out_dir {
        Some(d) => d.to_path_buf(),
        None => tempdir_in_target(),
    };
    let safe_name: String = plugin_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let src = build_dir.join(format!("plugin_{}.rs", safe_name));
    let cdylib_name = format!("bee_plugin_test_{}", safe_name);
    let output = build_dir.join(cdylib_lib_filename(&cdylib_name));
    let plugin_name_lit = plugin_name.replace('"', "\\\"");
    let template = r#"
use bee_plugin_sdk::{
    AdapterDescriptor, Factory, PluginHandle, PluginManifest, PluginName, PluginResult,
};
use std::collections::HashMap;
use std::sync::Arc;

pub struct TestFactory;

impl Factory for TestFactory {
    fn manifest() -> PluginManifest {
        PluginManifest {
            name: PluginName("__PLUGIN_NAME__".into()),
            feature_version: "0.0.1".into(),
            abi_version: "v1".into(),
            adapters: vec![AdapterDescriptor {
                name: "subscribe".into(),
                is_input: true,
            }],
            handlers: vec![],
        }
    }
    fn init() -> PluginResult<PluginHandle> {
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: Arc::new(()),
            input_adapters: HashMap::new(),
            output_adapters: HashMap::new(),
            handlers: HashMap::new(),
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(TestFactory);
"#;
    let src_text = template.replace("__PLUGIN_NAME__", &plugin_name_lit);
    std::fs::write(&src, src_text).ok()?;
    let status = std::process::Command::new("rustc")
        .args([
            "--crate-type=cdylib",
            "--edition=2021",
            &format!("--extern=bee_plugin_sdk={}", sdk_rlib.display()),
            &format!("-L"),
            &format!("{}", deps.display()),
            "-o",
            &output.display().to_string(),
        ])
        .arg(&src)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    if !output.exists() {
        return None;
    }
    Some(output)
}

fn cdylib_lib_filename(lib_name: &str) -> String {
    let prefix = std::env::consts::DLL_PREFIX;
    let suffix = std::env::consts::DLL_SUFFIX;
    format!("{prefix}{lib_name}{suffix}")
}

#[cfg(test)]
fn tempdir_in_target() -> std::path::PathBuf {
    let target = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target");
    let dir = if target.is_dir() {
        target.join("plugin-registry-test")
    } else {
        std::env::temp_dir().join("bee-plugin-registry-test")
    };
    let _ = std::fs::create_dir_all(&dir);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let sub = dir.join(format!("build-{pid}-{nanos}"));
    let _ = std::fs::create_dir_all(&sub);
    sub
}
