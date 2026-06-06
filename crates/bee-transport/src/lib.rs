//! `bee-transport` — BRP 传输层 (Layer 1 of the BRP protocol stack).
//!
//! 负责 Node-to-Node 的 TCP 连接生命周期、字节流到 Frame 的拆分与粘合。
//! 协议细节见 [`docs/architecture.md` §6.1](https://example.invalid/architecture#6-brp-协议分层).
//!
//! S00 阶段仅占位;S02 起实现 `Listener` / `Connection` / `TcpFramed`。

pub struct TcpFramed;
