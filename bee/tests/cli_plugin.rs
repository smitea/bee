use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_plugin_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("bee-plugin-cli-{nonce}"));
    fs::create_dir(&dir).unwrap();
    dir
}

fn plugin_artifact() -> PathBuf {
    let filename = if cfg!(target_os = "macos") {
        "libbee_plugin_perf_fib.dylib"
    } else if cfg!(target_os = "windows") {
        "bee_plugin_perf_fib.dll"
    } else {
        "libbee_plugin_perf_fib.so"
    };
    PathBuf::from(env!("CARGO_BIN_EXE_bee"))
        .parent()
        .expect("bee binary has a profile directory")
        .join("deps")
        .join(filename)
}

fn run_bee(args: &[&str], plugin_dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bee"))
        .args(args)
        .env("BEE_PLUGIN_DIR", plugin_dir)
        .output()
        .expect("failed to execute bee binary")
}

#[test]
fn plugin_list_reports_plugin_copied_into_plugin_directory() {
    let source = plugin_artifact();
    assert!(
        source.exists(),
        "missing test plugin at {}",
        source.display()
    );
    let dir = temp_plugin_dir();
    let destination = dir.join(if cfg!(target_os = "macos") {
        "libbee_plugin_fake.dylib"
    } else if cfg!(target_os = "windows") {
        "bee_plugin_fake.dll"
    } else {
        "libbee_plugin_fake.so"
    });
    fs::copy(&source, &destination).unwrap();

    let output = run_bee(&["plugin", "list"], &dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(stdout.contains("bee-plugin-perf-fib"), "stdout={stdout:?}");
    assert!(stdout.contains("hash="), "stdout={stdout:?}");
    assert!(stdout.contains("refcount=0"), "stdout={stdout:?}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn plugin_inspect_reports_hash_and_claimed_abi_version() {
    let source = plugin_artifact();
    assert!(
        source.exists(),
        "missing test plugin at {}",
        source.display()
    );
    let dir = temp_plugin_dir();

    let output = run_bee(&["plugin", "inspect", source.to_str().unwrap()], &dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(stdout.contains("hash="), "stdout={stdout:?}");
    assert!(stdout.contains("abi_version=v1"), "stdout={stdout:?}");
    let _ = fs::remove_dir_all(dir);
}
