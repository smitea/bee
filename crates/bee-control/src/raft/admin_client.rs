//! `AdminClient` — the `bee --connect <addr>` side of
//! the admin RPC. Connects to a Node's `AdminServer`,
//! sends a serialized `AdminRequest`, reads back the
//! matching `AdminResponse`.
//!
//! Wire format: `Frame { message_type = Admin,
//! body = bincode(AdminRequest) }` in, `Frame { ...,
//! body = bincode(AdminResponse) }` out.

use std::net::SocketAddr;

use bee_codec::{Frame, MessageType};
use bee_transport::Connection;
use thiserror::Error;

use crate::raft::admin_protocol::{AdminRequest, AdminResponse};

#[derive(Debug, Error)]
pub enum AdminError {
    #[error("io: {0}")]
    Io(String),
    #[error("bincode: {0}")]
    Bincode(String),
    #[error("server returned error: {0}")]
    ServerError(String),
}

pub struct AdminClient {
    addr: SocketAddr,
    conn: Connection,
}

impl AdminClient {
    pub async fn connect(addr: SocketAddr) -> Result<Self, AdminError> {
        let conn = Connection::connect(&addr.to_string())
            .await
            .map_err(|e| AdminError::Io(format!("connect: {e}")))?;
        Ok(Self { addr, conn })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn call(&mut self, req: AdminRequest) -> Result<AdminResponse, AdminError> {
        let body = bincode::serialize(&req)
            .map_err(|e| AdminError::Bincode(format!("encode: {e}")))?;
        let frame = Frame::new(MessageType::Admin, 0, body);
        self.conn
            .send_frame(&frame)
            .await
            .map_err(|e| AdminError::Io(format!("send: {e}")))?;
        let resp = self
            .conn
            .recv_frame()
            .await
            .map_err(|e| AdminError::Io(format!("recv: {e}")))?;
        if resp.message_type != MessageType::Admin {
            return Err(AdminError::Io(format!(
                "expected MessageType::Admin, got {:?}",
                resp.message_type
            )));
        }
        let response: AdminResponse = bincode::deserialize(&resp.body)
            .map_err(|e| AdminError::Bincode(format!("decode: {e}")))?;
        if let AdminResponse::Error(msg) = &response {
            return Err(AdminError::ServerError(msg.clone()));
        }
        Ok(response)
    }
}
