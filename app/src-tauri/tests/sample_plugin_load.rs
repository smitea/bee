//! End-to-end integration test for the "Reload from disk" flow.
//!
//! Builds the `sample-kline` plugin, copies the resulting `.dylib`
//! into a temp directory, and verifies that:
//!
//! - `PluginRegistry::scan_directory` finds it and returns a
//!   summary whose logical name is `sample-kline`.
//! - `plugin_schema("sample-kline")` returns a schema containing
//!   the `kline` input adapter, the `emit` output adapter, and the
//!   `ema` handler.
//! - Loading the same `.dylib` twice yields the same handle (id
//!   derived from the content hash, stable across reloads).
//!
//! Run with:
//!
//! ```bash
//! cd app/src-tauri && cargo test --test sample_plugin_load -- --nocapture
//! ```

use std::path::PathBuf;
use std::process::Command;

use app_lib::commands::plugins::{
    plugin_default_dir, plugin_schema, plugin_scan_directory,
};
use app_lib::plugin_registry::PluginRegistry;

fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or(crate_dir)
}

fn cdylib_filename(lib_name: &str) -> String {
    let prefix = std::env::consts::DLL_PREFIX;
    let suffix = std::env::consts::DLL_SUFFIX;
    format!("{prefix}{lib_name}{suffix}")
}

fn ensure_sample_plugin_built() -> PathBuf {
    let root = workspace_root();
    let target_release = root.join("target").join("release");
    let target_debug = root.join("target").join("debug");
    let release_lib = target_release.join(cdylib_filename("bee_plugin_sample_kline"));
    let debug_lib = target_debug.join(cdylib_filename("bee_plugin_sample_kline"));

    if release_lib.exists() {
        return release_lib;
    }
    if debug_lib.exists() {
        return debug_lib;
    }

    let status = Command::new("cargo")
        .arg("build")
        .current_dir(&root)
        .status()
        .expect("failed to spawn cargo build");
    assert!(status.success(), "cargo build failed");

    if debug_lib.exists() {
        return debug_lib;
    }
    panic!(
        "expected sample plugin cdylib at {}",
        debug_lib.display()
    );
}

fn copy_plugin_to_temp(source: &std::path::Path) -> (tempdir::TempDir, PathBuf) {
    let dir = tempdir::TempDir::new("bee-sample-plugin-test").expect("create tempdir");
    let dest = dir.path().join(source.file_name().expect("file name"));
    std::fs::copy(source, &dest).expect("copy cdylib into tempdir");
    (dir, dest)
}

#[test]
fn scan_directory_loads_sample_kline_plugin() {
    let source = ensure_sample_plugin_built();
    let (_dir, _lib) = copy_plugin_to_temp(&source);

    let registry = PluginRegistry::new();
    let summaries = registry.scan_directory(_dir.path());

    assert!(
        !summaries.is_empty(),
        "expected at least one summary, got 0"
    );
    let sample = summaries
        .iter()
        .find(|s| s.name == "sample-kline")
        .expect("sample-kline summary must be present after Reload from disk");
    assert_eq!(sample.version, "0.1.0");
    assert!(
        sample.adapters.iter().any(|a| a == "kline"),
        "sample-kline must advertise a kline adapter, got {:?}",
        sample.adapters
    );
    assert!(
        sample.adapters.iter().any(|a| a == "emit"),
        "sample-kline must advertise an emit adapter, got {:?}",
        sample.adapters
    );
    assert!(
        sample.handlers.iter().any(|h| h == "ema"),
        "sample-kline must advertise an ema handler, got {:?}",
        sample.handlers
    );
}

#[test]
fn plugin_schema_returns_kline_adapter_for_sample_kline() {
    let source = ensure_sample_plugin_built();
    let (dir, _lib) = copy_plugin_to_temp(&source);

    let summaries = plugin_scan_directory(dir.path().to_string_lossy().into_owned());
    assert!(
        summaries.iter().any(|s| s.name == "sample-kline"),
        "plugin_scan_directory must load sample-kline into the static registry"
    );

    let schema = plugin_schema("sample-kline".to_string());
    assert_eq!(schema.name, "sample-kline");
    let kline = schema
        .adapters
        .get("kline")
        .expect("kline adapter must be present in schema");
    let kline_type = kline
        .get("type")
        .and_then(|v| v.as_str())
        .expect("kline adapter must declare a type");
    assert_eq!(
        kline_type, "input",
        "kline adapter must be an input adapter"
    );
    let adapters_map = schema
        .adapters
        .as_object()
        .expect("schema adapters must be an object");
    assert!(
        adapters_map.contains_key("emit"),
        "emit adapter must be present in schema"
    );
}

#[test]
fn loading_sample_plugin_twice_yields_same_handle() {
    let source = ensure_sample_plugin_built();
    let (_dir, _lib) = copy_plugin_to_temp(&source);

    let registry = PluginRegistry::new();
    let first_id = registry.load(&source).expect("first load");
    let second_id = registry.load(&source).expect("second load");
    assert_eq!(
        first_id, second_id,
        "content-hash id must be stable across reloads"
    );
    let summaries = registry.list_summaries();
    let sample = summaries
        .iter()
        .find(|s| s.name == "sample-kline")
        .expect("sample-kline summary must be present");
    assert_eq!(
        sample.id, first_id,
        "summary.id must match the content-hash PluginId"
    );
}

#[test]
fn default_plugin_dir_resolves_to_home_bee_plugins() {
    let dir = plugin_default_dir();
    assert!(!dir.is_empty(), "default plugin dir must be non-empty");
    let contains_bee_plugins = dir.contains(".bee")
        && (dir.ends_with("/plugins") || dir.ends_with("\\plugins"));
    assert!(
        contains_bee_plugins,
        "default plugin dir must live under $HOME/.bee/plugins, got {dir}"
    );
}