use std::net::SocketAddr;

use bee_codec::Frame;
use bee_codec::MessageType;
use bee_transport::Connection;
use bee_transport::Listener;

#[tokio::test]
async fn bind_to_port_zero_assigns_a_real_local_port() {
    let listener = Listener::bind("127.0.0.1:0")
        .await
        .expect("bind to ephemeral port must succeed");
    let addr: SocketAddr = listener.local_addr();
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
    assert_ne!(addr.port(), 0, "OS must assign a non-zero port");
}

#[tokio::test]
async fn accept_returns_connection_for_incoming_tcp() {
    let listener = Listener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr();

    let server = tokio::spawn(async move {
        let mut conn = listener.accept().await.expect("accept must succeed");
        let frame = conn.recv_frame().await.expect("recv must succeed");
        conn.send_frame(&frame).await.expect("echo must succeed");
    });

    let mut conn = Connection::connect(&addr.to_string())
        .await
        .expect("client connect must succeed");
    let original = Frame::new(MessageType::Heartbeat, 1, b"ping".to_vec());
    conn.send_frame(&original).await.expect("send must succeed");
    let echoed = conn.recv_frame().await.expect("recv must succeed");
    assert_eq!(echoed, original);

    server.await.unwrap();
}
