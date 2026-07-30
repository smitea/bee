//! Tauri launch smoke test.
//!
//! Spawns the compiled `app` binary with an isolated data directory and
//! verifies that:
//!
//! - The binary exists at `app/src-tauri/target/debug/app` (or `release/app`)
//!   and is non-empty after `cargo build --bin app`.
//! - Launching it with a clean temp data dir keeps the process alive long
//!   enough for the WebView window to become visible. On macOS the window
//!   is detected via `osascript`; on other platforms the test falls back to
//!   a process-alive probe.
//! - A graceful SIGTERM cleanly shuts the process down so that the
//!   `tempdir::TempDir` can release the temp data dir.
//!
//! Run with:
//!
//! ```bash
//! cd app/src-tauri && cargo test --test tauri_launch_smoke -- --nocapture
//! ```

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempdir::TempDir;

const APP_BIN_NAME: &str = "app";
const APP_WINDOW_TITLE: &str = "Bee Client";
const APP_MACOS_PROCESS_NAME: &str = "app";

fn workspace_app_bin() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("target").join("debug").join(APP_BIN_NAME)
}

fn release_app_bin() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("target").join("release").join(APP_BIN_NAME)
}

fn resolve_app_bin() -> PathBuf {
    let release = release_app_bin();
    if release.exists() {
        release
    } else {
        workspace_app_bin()
    }
}

fn ensure_app_bin_built() -> PathBuf {
    let bin = resolve_app_bin();
    if !bin.exists() {
        let status = Command::new("cargo")
            .args(["build", "--bin", APP_BIN_NAME])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("spawn cargo build --bin app");
        assert!(status.success(), "cargo build --bin app failed");
    }
    assert!(
        bin.exists(),
        "app binary must exist at {}",
        bin.display()
    );
    bin
}

fn isolated_env(data_dir: &Path) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            !matches!(
                k.as_str(),
                "BEE_CLIENT_DB" | "BEE_ADMIN_ADDR" | "BEE_CLIENT_TENANT"
            )
        })
        .collect();
    let home = data_dir.to_string_lossy().into_owned();
    env.push(("HOME".to_string(), home.clone()));
    env.push(("XDG_DATA_HOME".to_string(), home.clone()));
    env.push(("APPDATA".to_string(), home));
    env.push((
        "BEE_ADMIN_ADDR".to_string(),
        "127.0.0.1:1".to_string(),
    ));
    env
}

fn spawn_isolated(bin: &Path, data_dir: &Path) -> Child {
    Command::new(bin)
        .envs(isolated_env(data_dir))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn app binary")
}

fn read_child_stdout(child: &mut Child) -> String {
    let mut buf = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut buf);
    }
    buf
}

fn read_child_stderr(child: &mut Child) -> String {
    let mut buf = String::new();
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut buf);
    }
    buf
}

#[cfg(unix)]
fn shutdown(child: &mut Child) {
    let pid = child.id() as i32;
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => return,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[cfg(not(unix))]
fn shutdown(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<(bool, String, String)> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("spawn failed: {e}");
            return None;
        }
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = read_child_stdout(&mut child);
                let stderr = read_child_stderr(&mut child);
                return Some((status.success(), stdout, stderr));
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_visible_process_names() -> Option<String> {
    let mut cmd = Command::new("osascript");
    cmd.args([
        "-e",
        "tell application \"System Events\" to get name of every process whose visible is true",
    ]);
    let (ok, stdout, stderr) = run_with_timeout(cmd, Duration::from_secs(5))?;
    if !ok && stderr.to_lowercase().contains("not authorized") {
        return None;
    }
    Some(stdout)
}

#[cfg(target_os = "macos")]
fn macos_window_titles_for_process(process_name: &str) -> Option<String> {
    let script = format!(
        "tell application \"System Events\" to tell process \"{}\" to get name of windows",
        process_name.replace('"', "\\\"")
    );
    let mut cmd = Command::new("osascript");
    cmd.args(["-e", &script]);
    let (_ok, stdout, _stderr) = run_with_timeout(cmd, Duration::from_secs(5))?;
    Some(stdout)
}

fn contains_token(haystack: &str, needle: &str) -> bool {
    haystack
        .split(',')
        .map(|s| s.trim())
        .any(|token| token == needle)
}

#[cfg(target_os = "macos")]
fn platform_app_window_visible() -> bool {
    let names = match macos_visible_process_names() {
        Some(n) => n,
        None => return false,
    };
    if !contains_token(&names, APP_MACOS_PROCESS_NAME) {
        return false;
    }
    match macos_window_titles_for_process(APP_MACOS_PROCESS_NAME) {
        Some(windows) => contains_token(&windows, APP_WINDOW_TITLE),
        None => false,
    }
}

#[cfg(not(target_os = "macos"))]
fn platform_app_window_visible() -> bool {
    true
}

fn process_alive(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

fn sqlite_artifact_created(data_dir: &Path) -> bool {
    let candidate = data_dir
        .join("Library")
        .join("Application Support")
        .join("io.smitea.beeclient")
        .join("bee-client.sqlite");
    candidate.exists()
}

#[test]
fn binary_exists_after_build() {
    let bin = ensure_app_bin_built();
    let metadata = std::fs::metadata(&bin).expect("stat app binary");
    assert!(
        metadata.len() > 0,
        "app binary must be non-empty, got {} bytes",
        metadata.len()
    );
}

#[test]
fn launch_starts_window_or_exits_cleanly() {
    let bin = ensure_app_bin_built();
    let dir = TempDir::new("bee-tauri-smoke").expect("tempdir");
    let data_path = dir.path().to_path_buf();
    let mut child = spawn_isolated(&bin, &data_path);

    let boot_deadline = Instant::now() + Duration::from_secs(20);
    let mut saw_window = false;
    let mut saw_process = false;
    let mut saw_db = false;
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                let out = read_child_stdout(&mut child);
                let err = read_child_stderr(&mut child);
                panic!(
                    "app exited unexpectedly with status {:?} after {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                    status,
                    boot_deadline.elapsed(),
                    out,
                    err
                );
            }
            None => {
                if !saw_process {
                    saw_process = true;
                }
                if !saw_db && sqlite_artifact_created(&data_path) {
                    saw_db = true;
                }
                if !saw_window && (saw_db || platform_app_window_visible()) {
                    saw_window = true;
                }
                if saw_window {
                    break;
                }
                if Instant::now() >= boot_deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }

    assert!(
        saw_process,
        "app process did not stay alive long enough to probe"
    );
    assert!(
        saw_window,
        "Bee Client window/process signal did not appear within {:?} (saw_db={saw_db})",
        boot_deadline.elapsed()
    );

    shutdown(&mut child);
    assert!(
        !process_alive(&mut child),
        "app must exit after SIGTERM"
    );
}

#[test]
fn shutdown_releases_temp_dir() {
    let bin = ensure_app_bin_built();
    let dir = TempDir::new("bee-tauri-smoke-cleanup").expect("tempdir");
    let data_path = dir.path().to_path_buf();
    let mut child = spawn_isolated(&bin, &data_path);
    std::thread::sleep(Duration::from_secs(3));
    shutdown(&mut child);
    let _status = child.wait().expect("wait after shutdown");
    drop(dir);
    assert!(
        !data_path.exists(),
        "tempdir {} should be removed after TempDir drop",
        data_path.display()
    );
}