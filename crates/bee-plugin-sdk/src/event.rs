//! S33 §1: production wire format for adapter events across the FFI.
//!
//! `Event` (from `bee-adapter`) crosses the `cdylib` boundary as
//! bincode-serialized bytes. The `EventBytes` struct is the FFI-
//! facing view: a `(ptr, len)` pair the host reads (and then
//! bincode-deserializes). Memory ownership: the producer allocates
//! the bytes (via `Vec<u8>::into_boxed_slice().leak()` or
//! `Box::into_raw`), the consumer reads them once, then frees
//! via the vtable's `close` or the same producer's allocator.

use bee_adapter::Event;

/// FFI-facing view of a serialized Event. The `ptr` is non-null
/// when `len > 0`; both fields are read-only from the consumer's
/// perspective. The producer is responsible for the bytes'
/// lifetime — see the vtable docs for the exact contract.
#[repr(C)]
pub struct EventBytes {
    pub ptr: *const u8,
    pub len: usize,
}

impl EventBytes {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };
}

/// Encode an `Event` to bincode bytes. The result is what crosses
/// the FFI boundary (the host reads it via `EventBytes`).
pub fn encode_event(event: &Event) -> Vec<u8> {
    bincode::serialize(event).expect("Event is always bincode-serializable")
}

/// Decode bincode bytes (as read from the FFI) back into an `Event`.
/// Returns `Err` if the bytes are malformed or the version field
/// (if added in the future) is incompatible.
pub fn decode_event(bytes: &[u8]) -> Result<Event, bincode::Error> {
    bincode::deserialize(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_event() {
        let event = Event {
            timestamp: 1_700_000_000_000,
            sequence: 42,
            payload: b"hello world".to_vec(),
        };
        let bytes = encode_event(&event);
        let decoded = decode_event(&bytes).expect("decode");
        assert_eq!(event, decoded);
    }

    #[test]
    fn roundtrip_empty_payload() {
        let event = Event {
            timestamp: 0,
            sequence: 0,
            payload: vec![],
        };
        let bytes = encode_event(&event);
        let decoded = decode_event(&bytes).expect("decode");
        assert_eq!(event, decoded);
    }

    #[test]
    fn roundtrip_large_payload() {
        // 1 MB payload
        let event = Event {
            timestamp: u64::MAX,
            sequence: u64::MAX,
            payload: vec![0xAB; 1_000_000],
        };
        let bytes = encode_event(&event);
        let decoded = decode_event(&bytes).expect("decode");
        assert_eq!(event, decoded);
    }

    #[test]
    fn decode_rejects_garbage() {
        let bytes = vec![0xFFu8; 4];
        let err = decode_event(&bytes);
        assert!(err.is_err(), "garbage must not decode");
    }

    #[test]
    fn empty_event_bytes_is_safe() {
        let eb = EventBytes::EMPTY;
        assert!(eb.ptr.is_null());
        assert_eq!(eb.len, 0);
    }
}
