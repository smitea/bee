//! S33 §2: per-adapter function pointer tables (vtables).
//!
//! Each vtable is `#[repr(C)]` so the layout is stable across the
//! FFI boundary. The host calls through the function pointers; the
//! plugin (in its `init()`) bundles the vtables into `PluginHandle`.
//!
//! ## Memory ownership across the FFI
//!
//! - `*const u8` / `*mut u8` + `len` is the canonical C ABI for
//!   variable-length bytes. The producer is responsible for the
//!   bytes' lifetime.
//! - For `next` (InputAdapter) and `emit` (OutputAdapter), the
//!   event bytes are bincode-encoded `Event` (see `event.rs`).
//! - For `handle` (Handler), the state and result are
//!   bincode-encoded plugin-specific types — the Handler is
//!   responsible for its own state schema.
//! - All `err_out` parameters are optional (may be null). When
//!   non-null, the plugin writes a bincode-encoded error string
//!   (UTF-8 bytes, NOT a C string) on failure. The host reads +
//!   frees (or ignores on success).

use crate::event::EventBytes;

/// Function pointer table for an `InputAdapter` instance.
/// All function pointers take a `ctx: *mut c_void` (the adapter's
/// per-instance state, allocated by `open`) as the first arg.
#[repr(C)]
pub struct InputAdapterVtable {
    /// Open the adapter with a config (bincode-encoded plugin-
    /// specific config blob). Returns a `*mut c_void` ctx for
    /// subsequent calls, or null on error (with `err_out` filled
    /// if non-null).
    pub open: unsafe extern "C" fn(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void,

    /// Pull the next event. Returns:
    /// - 1 if an event was produced (written to `*out`)
    /// - 0 for end-of-stream
    /// - -1 on error (with `*err_out` filled if non-null)
    /// The producer owns the bytes; the consumer must copy them
    /// out before the next call (or before `close`).
    pub next: unsafe extern "C" fn(
        ctx: *mut std::ffi::c_void,
        out: *mut EventBytes,
    ) -> i32,

    /// Close the adapter; free the ctx. Returns 0 on success.
    pub close: unsafe extern "C" fn(
        ctx: *mut std::ffi::c_void,
    ) -> i32,
}

#[repr(C)]
pub struct OutputAdapterVtable {
    pub open: unsafe extern "C" fn(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void,

    /// Emit one event. The host has already bincode-encoded the
    /// `Event` (see `event::encode_event`); the plugin
    /// bincode-decodes + processes. Returns 0 on success, -1 on
    /// error.
    pub emit: unsafe extern "C" fn(
        ctx: *mut std::ffi::c_void,
        event_ptr: *const u8,
        event_len: usize,
    ) -> i32,

    pub close: unsafe extern "C" fn(
        ctx: *mut std::ffi::c_void,
    ) -> i32,
}

#[repr(C)]
pub struct HandlerVtable {
    /// Compute `handler(state, event) -> (new_state, result)`.
    /// All blobs are bincode-encoded. Returns 0 on success.
    pub handle: unsafe extern "C" fn(
        state_ptr: *const u8,
        state_len: usize,
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        err_out: *mut EventBytes,
    ) -> i32,

    /// Initialize a fresh state blob. Returns 0 on success.
    pub init_state: unsafe extern "C" fn(
        out: *mut EventBytes,
    ) -> i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vtables_have_stable_layout() {
        // Compile-time / runtime sanity: the structs have non-zero
        // size and (because of #[repr(C)]) the field offsets are
        // predictable.
        assert!(std::mem::size_of::<InputAdapterVtable>() > 0);
        assert!(std::mem::size_of::<OutputAdapterVtable>() > 0);
        assert!(std::mem::size_of::<HandlerVtable>() > 0);
    }

    #[test]
    fn event_bytes_is_ffi_safe() {
        assert_eq!(
            std::mem::size_of::<EventBytes>(),
            std::mem::size_of::<*const u8>() + std::mem::size_of::<usize>(),
        );
    }
}
