use std::net::SocketAddr;

use bee_codec::Frame;
use bee_codec::MessageType;
use bee_transport::Listener;

#[tokio::test]
async fn recv_frame_reconstructs_from_split_tcp_writes() {
    let listener = Listener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr();

    let server = tokio::spawn(async move {
        let mut conn = listener.accept().await.unwrap();
        let frame = conn.recv_frame().await.unwrap();
        assert_eq!(frame.message_type, MessageType::Heartbeat);
        assert_eq!(frame.body, b"hello world".to_vec());
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let encoded = Frame::new(MessageType::Heartbeat, 0, b"hello world".to_vec()).encode();

    use tokio::io::AsyncWriteExt;
    stream.write_all(&encoded[..5]).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    stream.write_all(&encoded[5..15]).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    stream.write_all(&encoded[15..]).await.unwrap();
    stream.shutdown().await.unwrap();

    server.await.unwrap();
}
