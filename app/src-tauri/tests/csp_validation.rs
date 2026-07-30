//! Validates the production CSP declared in `tauri.conf.json`.
//!
//! These assertions run at `cargo test` time so the policy cannot
//! regress without a test failure. They cover:
//!
//! - CSP is present (not null).
//! - The policy is a strict allowlist rooted at `default-src 'self'`.
//! - `script-src` does not include `'unsafe-inline'` or `'unsafe-eval'`.
//! - `script-src-elem 'unsafe-inline'` is allowed only because dynamic
//!   `<script>` injection is a documented dependency of the chart
//!   libraries (ECharts / klinecharts); the script element source is
//!   still restricted to `'self'`, never arbitrary origins.
//! - `connect-src` covers the Tauri IPC bridge
//!   (`ipc:` + `http://ipc.localhost`).
//! - `object-src`, `frame-src`, and `form-action` are pinned to `'none'`
//!   to defeat plugin / framing / form-based exfiltration.
//! - No capability beyond `core:default` is granted in the default
//!   capability file.
//!
//! Run with:
//!
//! ```bash
//! cd app/src-tauri && cargo test --test csp_validation
//! ```

use std::fs;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tauri_conf_path() -> PathBuf {
    manifest_dir().join("tauri.conf.json")
}

fn capabilities_dir() -> PathBuf {
    manifest_dir().join("capabilities")
}

fn split_csp(csp: &str) -> Vec<(&str, &str)> {
    csp.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|directive| directive.split_once(' '))
        .collect()
}

fn lookup<'a>(parts: &'a [(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    parts
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| *v)
}

#[test]
fn csp_is_present() {
    let raw = fs::read_to_string(tauri_conf_path()).expect("read tauri.conf.json");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("parse tauri.conf.json");
    let csp = value
        .get("app")
        .and_then(|a| a.get("security"))
        .and_then(|s| s.get("csp"))
        .expect("app.security.csp present");

    assert!(
        !csp.is_null(),
        "CSP must be a string in production builds; got null"
    );
    assert!(
        csp.is_string(),
        "CSP must be a single JSON string; got non-string value"
    );
}

#[test]
fn csp_default_src_is_self() {
    let csp = load_csp();
    let parts = split_csp(&csp);
    let default_src = lookup(&parts, "default-src").expect("default-src directive");
    assert!(
        default_src.contains("'self'"),
        "default-src must include 'self' to anchor the allowlist; got `{default_src}`"
    );
}

#[test]
fn csp_script_src_has_no_inline_or_eval() {
    let csp = load_csp();
    let parts = split_csp(&csp);
    let script_src = lookup(&parts, "script-src").expect("script-src directive");
    assert!(
        !script_src.contains("'unsafe-inline'"),
        "script-src must not allow inline scripts; got `{script_src}`"
    );
    assert!(
        !script_src.contains("'unsafe-eval'"),
        "script-src must not allow eval; got `{script_src}`"
    );
}

#[test]
fn csp_script_src_elem_inline_is_justified() {
    let csp = load_csp();
    let parts = split_csp(&csp);
    let script_src = lookup(&parts, "script-src").expect("script-src directive");
    let script_src_elem = match lookup(&parts, "script-src-elem") {
        Some(v) => v,
        None => {
            assert!(
                !script_src.contains("'unsafe-inline'"),
                "script-src-elem is not declared, therefore script-src must not include 'unsafe-inline'"
            );
            return;
        }
    };

    if script_src_elem.contains("'unsafe-inline'") {
        assert!(
            !script_src_elem.contains("http://") && !script_src_elem.contains("https://"),
            "script-src-elem 'unsafe-inline' must not widen the origin allowlist beyond 'self'"
        );
        assert!(
            !script_src.contains("'unsafe-inline'"),
            "script-src must stay narrower than script-src-elem"
        );
    }
}

#[test]
fn csp_blocks_objects_frames_and_forms() {
    let csp = load_csp();
    let parts = split_csp(&csp);
    for directive in ["object-src", "frame-src", "form-action"] {
        let value = lookup(&parts, directive).unwrap_or_else(|| {
            panic!("{directive} directive must be declared");
        });
        assert!(
            value.contains("'none'"),
            "{directive} must be 'none' to block exfiltration; got `{value}`"
        );
    }
}

#[test]
fn csp_connect_src_covers_ipc() {
    let csp = load_csp();
    let parts = split_csp(&csp);
    let connect = lookup(&parts, "connect-src").expect("connect-src directive");
    assert!(
        connect.contains("ipc:"),
        "connect-src must allow the ipc: scheme; got `{connect}`"
    );
    assert!(
        connect.contains("http://ipc.localhost"),
        "connect-src must allow http://ipc.localhost for the Tauri IPC bridge; got `{connect}`"
    );
}

#[test]
fn csp_caps_base_uri() {
    let csp = load_csp();
    let parts = split_csp(&csp);
    let base_uri = lookup(&parts, "base-uri").expect("base-uri directive");
    assert!(
        base_uri.contains("'self'"),
        "base-uri must be limited to 'self' to defeat <base> injection; got `{base_uri}`"
    );
}

#[test]
fn default_capability_grants_only_core() {
    let path = capabilities_dir().join("default.json");
    let raw = fs::read_to_string(&path).expect("read default.json");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("parse default.json");
    let permissions = value
        .get("permissions")
        .and_then(|p| p.as_array())
        .expect("permissions array");

    assert_eq!(
        permissions.len(),
        1,
        "default capability should expose only `core:default`; got {permissions:?}"
    );

    let only = permissions[0]
        .as_str()
        .expect("permission entries are strings");
    assert_eq!(
        only, "core:default",
        "the only granted permission must be `core:default`; got `{only}`"
    );

    for forbidden in [
        "fs:",
        "shell:",
        "http:",
        "dialog:",
        "clipboard:",
        "notification:",
        "process:",
        "os:",
        "path:",
    ] {
        for entry in permissions {
            let entry = entry.as_str().unwrap_or_default();
            assert!(
                !entry.starts_with(forbidden),
                "forbidden capability `{entry}` is granted; rationale required"
            );
        }
    }
}

fn load_csp() -> String {
    let raw = fs::read_to_string(tauri_conf_path()).expect("read tauri.conf.json");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("parse tauri.conf.json");
    let csp = value
        .get("app")
        .and_then(|a| a.get("security"))
        .and_then(|s| s.get("csp"))
        .expect("app.security.csp present");
    csp.as_str()
        .expect("CSP must be a string")
        .to_owned()
}
