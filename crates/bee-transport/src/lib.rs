//! `bee-transport` — BRP 传输层 (Layer 1 of the BRP protocol stack).
//!
//! 负责 Node-to-Node 的 TCP 连接生命周期、字节流到 Frame 的拆分与粘合。
//! 协议细节见 [`docs/architecture.md` §6.1](https://example.invalid/architecture#6-brp-协议分层).
//!
//! ## 类型
//! - [`Listener`] — 服务端,接受 TCP 连接
//! - [`Connection`] — 单条 TCP 连接,可发送/接收完整 [`Frame`]
//! - [`Framed`] — `Connection` 内部使用的帧分帧器,处理 TCP 粘包/半包
//! - [`TransportError`] — 统一错误类型,聚合 `std::io::Error` 与 `CodecError`

use std::io;
use std::net::SocketAddr;

use bytes::{Buf, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use bee_codec::{Frame, HEADER_LEN};

#[derive(Debug)]
pub enum TransportError {
    Io(io::Error),
    Codec(bee_codec::CodecError),
    ConnectionClosed,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Io(e) => write!(f, "io error: {e}"),
            TransportError::Codec(e) => write!(f, "codec error: {e}"),
            TransportError::ConnectionClosed => write!(f, "connection closed by peer"),
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TransportError::Io(e) => Some(e),
            TransportError::Codec(e) => Some(e),
            TransportError::ConnectionClosed => None,
        }
    }
}

impl From<io::Error> for TransportError {
    fn from(e: io::Error) -> Self {
        TransportError::Io(e)
    }
}

impl From<bee_codec::CodecError> for TransportError {
    fn from(e: bee_codec::CodecError) -> Self {
        TransportError::Codec(e)
    }
}

pub struct Listener {
    inner: TcpListener,
}

impl Listener {
    pub async fn bind(addr: &str) -> Result<Self, TransportError> {
        let inner = TcpListener::bind(addr).await?;
        Ok(Self { inner })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.inner
            .local_addr()
            .expect("local_addr is valid after successful bind")
    }

    pub async fn accept(&self) -> Result<Connection, TransportError> {
        let (stream, _peer) = self.inner.accept().await?;
        Ok(Connection::new(stream))
    }
}

pub struct Connection {
    framed: Framed,
}

impl Connection {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            framed: Framed::new(stream),
        }
    }

    pub async fn connect(addr: &str) -> Result<Self, TransportError> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self::new(stream))
    }

    pub async fn send_frame(&mut self, frame: &Frame) -> Result<(), TransportError> {
        self.framed.send_frame(frame).await
    }

    pub async fn recv_frame(&mut self) -> Result<Frame, TransportError> {
        self.framed.recv_frame().await
    }
}

pub struct Framed {
    stream: TcpStream,
    read_buf: BytesMut,
}

impl Framed {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            read_buf: BytesMut::new(),
        }
    }

    pub async fn send_frame(&mut self, frame: &Frame) -> Result<(), TransportError> {
        let bytes = frame.encode();
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn recv_frame(&mut self) -> Result<Frame, TransportError> {
        loop {
            if self.read_buf.len() >= HEADER_LEN {
                let mut bl = [0u8; 4];
                bl.copy_from_slice(&self.read_buf[11..15]);
                let body_length = u32::from_be_bytes(bl) as usize;
                let total = HEADER_LEN + body_length;
                if self.read_buf.len() >= total {
                    let (frame, consumed) = Frame::decode(&self.read_buf)?;
                    self.read_buf.advance(consumed);
                    return Ok(frame);
                }
            }
            let mut tmp = [0u8; 4096];
            let n = self.stream.read(&mut tmp).await?;
            if n == 0 {
                return Err(TransportError::ConnectionClosed);
            }
            self.read_buf.extend_from_slice(&tmp[..n]);
        }
    }
}
