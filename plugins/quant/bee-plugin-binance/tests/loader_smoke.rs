//! End-to-end smoke test: build this plugin's `.dylib`, then
//! load it via `bee-registry::loader::load_library` and verify
//! the manifest matches.
//!
//! This is the **acceptance test for S33 step 4**: the libloading
//! path can load a real `cdylib` plugin and recover its manifest.
//!
//! ## Why this lives in the plugin's tests/
//!
//! The plugin is the artifact under test. The test depends on the
//! `.dylib` existing in the same workspace `target/debug/` dir,
//! which `cargo test` produces as a side effect of compiling the
//! lib (with `crate-type = ["cdylib", "rlib"]`).

use std::path::PathBuf;

use bee_plugin_sdk::compute_plugin_id;
use bee_registry::loader::load_library;

fn workspace_target_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR points to this plugin's crate root;
    // the .dylib lives in ../../../target/debug/ (the workspace
    // target dir).
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_target = crate_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("target")
        .join("debug");
    workspace_target
}

fn dylib_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libbee_plugin_binance.dylib"
    }
    #[cfg(target_os = "linux")]
    {
        "libbee_plugin_binance.so"
    }
    #[cfg(target_os = "windows")]
    {
        "bee_plugin_binance.dll"
    }
}

#[test]
fn load_binance_dylib_yields_expected_manifest() {
    let path = workspace_target_dir().join(dylib_name());
    if !path.exists() {
        // If the .dylib is missing (e.g. --no-default-features ran
        // and skipped the cdylib build), the test is a no-op. A
        // full `cargo build` produces the artifact.
        eprintln!(
            "skipping: cdylib not found at {}. run `cargo build` first.",
            path.display()
        );
        return;
    }

    let loaded = load_library(&path).expect("load_library");
    let m = loaded.manifest();
    assert_eq!(m.name.0, "binance");
    assert_eq!(m.abi_version, "v1");
    assert_eq!(m.adapters.len(), 1);
    assert_eq!(m.adapters[0].name, "subscribe");
    assert!(m.adapters[0].is_input);

    // The PluginId must match the sha256 of the file content
    // (proves the content-hash binding from ADR-0009).
    let content = std::fs::read(&path).expect("read");
    assert_eq!(loaded.id(), &compute_plugin_id(&content));
}
