use std::process::Command;

use bee_transport::Listener;

#[tokio::test]
async fn bee_echo_subcommand_round_trips_a_heartbeat_frame() {
    let listener = Listener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr();
    let addr_str = addr.to_string();

    let server = tokio::spawn(async move {
        let mut conn = listener.accept().await.unwrap();
        let frame = conn.recv_frame().await.unwrap();
        conn.send_frame(&frame).await.unwrap();
    });

    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_bee"))
            .arg("echo")
            .arg(&addr_str)
            .output()
            .expect("failed to execute bee binary")
    })
    .await
    .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "bee echo exited with non-zero status: stdout={stdout:?} stderr={stderr:?}"
    );
    assert_eq!(stdout.trim(), "ok", "expected `ok` on stdout, got {stdout:?}");

    server.await.unwrap();
}

#[tokio::test]
async fn bee_echo_subcommand_fails_cleanly_on_missing_addr() {
    let output = tokio::task::spawn_blocking(|| {
        Command::new(env!("CARGO_BIN_EXE_bee"))
            .arg("echo")
            .output()
            .expect("failed to execute bee binary")
    })
    .await
    .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("echo requires <addr>"),
        "expected usage error in stderr, got: {stderr}"
    );
}
