//! Error type for GUI-side failures. Every variant logs via `tracing::error!`
//! with full source-chain context.

use std::io;
use std::net::SocketAddr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GuiError {
    #[error("Connect failed to {addr} after {attempts} attempts: {last_err}")]
    Connect {
        addr: SocketAddr,
        attempts: u32,
        last_err: String,
    },

    #[error("RPC timeout after {elapsed_ms}ms (rpc={rpc})")]
    Timeout { rpc: &'static str, elapsed_ms: u64 },

    #[error("Server returned error: {msg}")]
    RpcServer { msg: String },

    #[error("Wire {kind} error: {detail}")]
    Wire { kind: WireErrKind, detail: String },

    #[error("I/O error: {source}")]
    Io {
        #[source]
        source: io::Error,
    },

    #[error("Connection lost (last seen {last_seen_ms}ms ago)")]
    ConnectionLost { last_seen_ms: u64 },

    #[error("Cancelled by user")]
    Cancelled,
}

#[derive(Debug, Clone, Copy)]
pub enum WireErrKind {
    Decode,
    Encode,
}

impl std::fmt::Display for WireErrKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode => write!(f, "decode"),
            Self::Encode => write!(f, "encode"),
        }
    }
}

pub struct CallContext {
    pub id: u64,
    pub rpc_kind: &'static str,
    pub addr: SocketAddr,
    pub started_at_ms: u64,
    pub elapsed_ms: u64,
    pub attempt: u32,
    pub conn_state: &'static str,
}

pub fn log_rpc_failure(ctx: &CallContext, err: &GuiError) {
    let chain: Vec<String> = std::iter::successors(Some(err as &dyn std::error::Error), |e| e.source())
        .map(|e| e.to_string())
        .collect();
    tracing::error!(
        target: "bee_gui.rpc",
        call_id = ctx.id,
        rpc = %ctx.rpc_kind,
        addr = %ctx.addr,
        started_at_ms = ctx.started_at_ms,
        elapsed_ms = ctx.elapsed_ms,
        attempt = ctx.attempt,
        connection_state = %ctx.conn_state,
        err.kind = ?err,
        err.detail = %err,
        err.chain = ?chain,
        "RPC call failed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_error_chain_io() {
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        let err = GuiError::Io { source: io_err };
        let display = format!("{}", err);
        assert!(
            display.contains("refused"),
            "Display must include source chain: got {}",
            display
        );
    }

    #[test]
    fn gui_error_chain_rpc_server() {
        let err = GuiError::RpcServer {
            msg: "tenant=0 not authorized".into(),
        };
        let display = format!("{}", err);
        assert!(display.contains("tenant=0"));
    }

    #[test]
    fn wire_err_kind_display() {
        assert_eq!(WireErrKind::Decode.to_string(), "decode");
        assert_eq!(WireErrKind::Encode.to_string(), "encode");
    }

    #[test]
    fn log_rpc_failure_does_not_panic() {
        let ctx = CallContext {
            id: 1,
            rpc_kind: "Ping",
            addr: "127.0.0.1:10001".parse().unwrap(),
            started_at_ms: 0,
            elapsed_ms: 100,
            attempt: 1,
            conn_state: "Connected",
        };
        let err = GuiError::Timeout {
            rpc: "Ping",
            elapsed_ms: 100,
        };
        log_rpc_failure(&ctx, &err);
    }
}