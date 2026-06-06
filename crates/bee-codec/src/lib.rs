//! `bee-codec` — BRP 编解码层 (Layer 2 of the BRP protocol stack).
//!
//! 提供 `Frame` 结构、`MessageType` 枚举、bincode 序列化辅助。
//! 协议格式见 [`docs/architecture.md` §7](https://example.invalid/architecture#7-二进制报文格式).
//!
//! S00 阶段仅占位;S01 起实现 `Frame` 的 encode / decode。

pub struct Frame;
