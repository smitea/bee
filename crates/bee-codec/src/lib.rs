//! `bee-codec` — BRP 编解码层 (Layer 2 of the BRP protocol stack).
//!
//! 提供 `Frame` 结构、`MessageType` 枚举、bincode 序列化辅助。
//! 协议格式见 [`docs/architecture.md` §7](https://example.invalid/architecture#7-二进制报文格式).
//!
//! S00 阶段仅占位;S01 起实现 `Frame` 的 encode / decode。

pub const MAGIC: [u8; 2] = [0x42, 0x45];
pub const HEADER_LEN: usize = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    Heartbeat = 0x01,
    DataPacket = 0x02,
    StealTask = 0x03,
}

impl MessageType {
    pub fn from_u8(b: u8) -> Result<Self, CodecError> {
        match b {
            0x01 => Ok(MessageType::Heartbeat),
            0x02 => Ok(MessageType::DataPacket),
            0x03 => Ok(MessageType::StealTask),
            other => Err(CodecError::UnknownMessageType(other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    BufferTooShort { needed: usize, got: usize },
    BadMagic { expected: [u8; 2], got: [u8; 2] },
    UnknownMessageType(u8),
    BodyLengthMismatch { declared: usize, available: usize },
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::BufferTooShort { needed, got } => write!(
                f,
                "buffer too short: need at least {needed} bytes, got {got}"
            ),
            CodecError::BadMagic { expected, got } => write!(
                f,
                "bad magic: expected {:02X?}, got {:02X?}",
                expected, got
            ),
            CodecError::UnknownMessageType(b) => {
                write!(f, "unknown message type: 0x{b:02X}")
            }
            CodecError::BodyLengthMismatch { declared, available } => write!(
                f,
                "body length mismatch: declared {declared} bytes, only {available} available"
            ),
        }
    }
}

impl std::error::Error for CodecError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub magic: [u8; 2],
    pub message_type: MessageType,
    pub request_id: u64,
    pub body: Vec<u8>,
}

impl Frame {
    pub fn new(message_type: MessageType, request_id: u64, body: Vec<u8>) -> Self {
        Self {
            magic: MAGIC,
            message_type,
            request_id,
            body,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(HEADER_LEN + self.body.len());
        buf.extend_from_slice(&self.magic);
        buf.push(self.message_type as u8);
        buf.extend_from_slice(&self.request_id.to_be_bytes());
        buf.extend_from_slice(&(self.body.len() as u32).to_be_bytes());
        buf.extend_from_slice(&self.body);
        buf
    }

    pub fn decode(bytes: &[u8]) -> Result<(Frame, usize), CodecError> {
        if bytes.len() < HEADER_LEN {
            return Err(CodecError::BufferTooShort {
                needed: HEADER_LEN,
                got: bytes.len(),
            });
        }

        let magic = [bytes[0], bytes[1]];
        if magic != MAGIC {
            return Err(CodecError::BadMagic {
                expected: MAGIC,
                got: magic,
            });
        }

        let message_type = MessageType::from_u8(bytes[2])?;

        let mut rid = [0u8; 8];
        rid.copy_from_slice(&bytes[3..11]);
        let request_id = u64::from_be_bytes(rid);

        let mut bl = [0u8; 4];
        bl.copy_from_slice(&bytes[11..15]);
        let body_length = u32::from_be_bytes(bl) as usize;

        let available_body = bytes.len() - HEADER_LEN;
        if body_length > available_body {
            return Err(CodecError::BodyLengthMismatch {
                declared: body_length,
                available: available_body,
            });
        }

        let body = bytes[HEADER_LEN..HEADER_LEN + body_length].to_vec();
        Ok((
            Frame {
                magic,
                message_type,
                request_id,
                body,
            },
            HEADER_LEN + body_length,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_produces_15_bytes_for_empty_body() {
        let frame = Frame::new(MessageType::Heartbeat, 0, Vec::new());
        let bytes = frame.encode();
        assert_eq!(bytes.len(), 15, "header alone must be 15 bytes");
        assert_eq!(&bytes[0..2], &MAGIC, "first 2 bytes must be the BRP magic");
        assert_eq!(bytes[2], MessageType::Heartbeat as u8);
        assert_eq!(&bytes[3..11], &0u64.to_be_bytes(), "request_id is big-endian u64");
        assert_eq!(&bytes[11..15], &0u32.to_be_bytes(), "body_length is big-endian u32");
    }

    #[test]
    fn encode_produces_header_plus_body_length() {
        let body = vec![0xAA, 0xBB, 0xCC, 0xDD];
        let frame = Frame::new(MessageType::DataPacket, 0x0102030405060708, body.clone());
        let bytes = frame.encode();
        assert_eq!(bytes.len(), 15 + body.len());
        assert_eq!(&bytes[15..], &body[..]);
    }

    #[test]
    fn decode_round_trip_empty_body() {
        let original = Frame::new(MessageType::Heartbeat, 0, Vec::new());
        let bytes = original.encode();
        let (decoded, consumed) = Frame::decode(&bytes).expect("decode should succeed");
        assert_eq!(consumed, bytes.len(), "consumed must equal encoded length");
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_round_trip_with_body() {
        let body = vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE];
        let original = Frame::new(MessageType::StealTask, 0xFFFF_FFFF_FFFF_FFFF, body);
        let bytes = original.encode();
        let (decoded, consumed) = Frame::decode(&bytes).expect("decode should succeed");
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_reports_consumed_bytes_when_extra_trailing_data_present() {
        let original = Frame::new(MessageType::Heartbeat, 42, Vec::new());
        let mut bytes = original.encode();
        bytes.extend_from_slice(&[0x00, 0x00, 0x00]);
        let (_decoded, consumed) = Frame::decode(&bytes).expect("decode should succeed");
        assert_eq!(consumed, 15, "decoder must stop at frame boundary, not over-consume");
    }

    #[test]
    fn decode_empty_buffer_returns_buffer_too_short() {
        let err = Frame::decode(&[]).expect_err("empty buffer must fail");
        assert_eq!(err, CodecError::BufferTooShort { needed: 15, got: 0 });
    }

    #[test]
    fn decode_partial_header_returns_buffer_too_short() {
        let err = Frame::decode(&[0x42, 0x45, 0x01, 0x00, 0x00]).expect_err("14 bytes must fail");
        assert_eq!(err, CodecError::BufferTooShort { needed: 15, got: 5 });
    }

    #[test]
    fn decode_bad_magic_returns_bad_magic_error() {
        let mut bytes = Frame::new(MessageType::Heartbeat, 0, Vec::new()).encode();
        bytes[0] = 0x00;
        bytes[1] = 0x00;
        let err = Frame::decode(&bytes).expect_err("bad magic must fail");
        assert_eq!(err, CodecError::BadMagic { expected: MAGIC, got: [0x00, 0x00] });
    }

    #[test]
    fn decode_body_length_mismatch_returns_error() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(MessageType::Heartbeat as u8);
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&100u32.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 5]);
        let err = Frame::decode(&bytes).expect_err("declared body > available must fail");
        assert_eq!(
            err,
            CodecError::BodyLengthMismatch { declared: 100, available: 5 }
        );
    }

    #[test]
    fn decode_unknown_message_type_returns_error() {
        let mut bytes = Frame::new(MessageType::Heartbeat, 0, Vec::new()).encode();
        bytes[2] = 0xFE;
        let err = Frame::decode(&bytes).expect_err("unknown message type must fail");
        assert_eq!(err, CodecError::UnknownMessageType(0xFE));
    }

    #[test]
    fn message_type_known_values_round_trip() {
        for mt in [MessageType::Heartbeat, MessageType::DataPacket, MessageType::StealTask] {
            let parsed = MessageType::from_u8(mt as u8).expect("known type must parse");
            assert_eq!(parsed, mt);
        }
    }

    #[test]
    fn message_type_unknown_value_returns_error() {
        let err = MessageType::from_u8(0x00).expect_err("0x00 is reserved");
        assert_eq!(err, CodecError::UnknownMessageType(0x00));
        let err = MessageType::from_u8(0x99).expect_err("0x99 is unassigned");
        assert_eq!(err, CodecError::UnknownMessageType(0x99));
    }
}
