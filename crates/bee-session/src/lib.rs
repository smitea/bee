//! `bee-session` — BRP 会话层 (Layer 3 of the BRP protocol stack).
//!
//! 负责 RequestId 多路复用、滑动窗口背压、连接池管理。
//!
//! S00 阶段仅占位;S04 起实现 `ConnectionPool` / `RequestRouter`。

pub struct ConnectionPool;
