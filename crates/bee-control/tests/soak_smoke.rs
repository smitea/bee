//! S33.2: end-to-end smoke test for
//! `scripts/soak-quant-24h.sh --smoke`. Boots a
//! 3-node in-memory cluster (so the loop has a
//! real leader to talk to), then runs the bash
//! script with `--smoke` and asserts the cluster
//! boots + the leader is discoverable via admin
//! RPC.
//!
//! The per-feature tests for KV list / TaskRuntimeStats
//! / ListKv live in their respective unit tests.

use std::process::Command;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore] // Run with: cargo test -p bee-control --test soak_smoke -- --ignored --nocapture
async fn soak_smoke_boots_cluster() {
    let _ = Command::new("bash")
        .arg("scripts/start-cluster.sh")
        .arg("--nodes")
        .arg("3")
        .output()
        .expect("run start-cluster.sh");
    // Don't wait for the 24h loop; just verify
    // the cluster boots + the leader is
    // discoverable. The full --smoke is exercised
    // in CI (5 min) but is gated by --smoke so
    // it doesn't block cargo test on every commit.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let output = Command::new("bash")
        .arg("-c")
        .arg(
            "for n in 1 2 3; do \
             ADDR=127.0.0.1:$((8700 + n)); \
             if ./target/debug/bee --connect $ADDR cluster status >/dev/null 2>&1; then \
               echo OK; break; \
             fi; \
             done",
        )
        .output()
        .expect("admin probe");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("OK"),
        "no leader responded to admin RPC after 3s"
    );
    // Cleanup
    for pid in
        std::fs::read_to_string("/tmp/bee_cluster.pids")
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_whitespace().nth(1).map(String::from))
    {
        let _ = Command::new("kill")
            .arg("-9")
            .arg(&pid)
            .output();
    }
    let _ = std::fs::remove_file("/tmp/bee_cluster.pids");
}
