//! S33.6.x (architecture-review #3): the
//! `Kv` port + adapters.
//!
//! Plugins that need per-stream state
//! (e.g. Producer HWM) used to call
//! hand-written `kv_get` / `kv_put`
//! helpers backed by a process-global
//! `OnceLock<Mutex<HashMap>>`. The shape
//! was shallow: 6 fns, the interface
//! matched the implementation, two
//! plugins had drifted between
//! `OnceLock` and `LazyLock`.
//!
//! This module introduces a **port**
//! (`Kv` trait) at the seam + two
//! **adapters**:
//!
//! - `InProcessKv` — a process-global
//!   `HashMap<String, Vec<u8>>` guarded
//!   by `Mutex`. For tests + plugin MVP
//!   (replaces the hand-written
//!   `kv_stub` in 2 plugins).
//! - `HostKv` — wraps the `BeeHostV1`
//!   FFI function pointers
//!   (`kv_get` / `kv_put`). For
//!   production: the plugin's KV writes
//!   go to the host's cluster KV.
//!
//! The two adapters justify the seam
//! (per `LANGUAGE.md`: one adapter =
//! hypothetical seam; two = real one).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The port: KV access. Both adapters
/// implement this; the plugin holds an
/// `Arc<dyn Kv>` and calls `.get` / `.put`
/// through the trait.
pub trait Kv: Send + Sync + 'static {
    /// Read a value by key. Returns
    /// `None` if the key is unset.
    fn get(&self, key: &str) -> Option<Vec<u8>>;

    /// Write a value (overwrites).
    /// Returns `()` on success.
    fn put(&self, key: &str, value: Vec<u8>);
}

/// In-process adapter. Process-global
/// `HashMap` guarded by `Mutex`. Used
/// for tests + the plugin MVP (when
/// the host's `BeeHostV1` doesn't
/// provide a `kv_get`/`kv_put`).
pub struct InProcessKv {
    inner: &'static Mutex<HashMap<String, Vec<u8>>>,
}

impl InProcessKv {
    /// Create a new in-process KV adapter.
    /// Each call returns a handle to the
    /// same process-global map (so
    /// multiple adapters share state —
    /// matching the prior `kv_stub`
    /// semantics where the HWM is shared
    /// across plugin instances).
    pub fn new() -> Arc<Self> {
        static KV: OnceLock<Mutex<HashMap<String, Vec<u8>>>> =
            OnceLock::new();
        let inner = KV.get_or_init(|| Mutex::new(HashMap::new()));
        Arc::new(Self { inner })
    }
}

impl Default for InProcessKv {
    fn default() -> Self {
        // The `Default` impl returns a
        // **fresh** per-instance KV
        // (no static). Useful for tests
        // that want per-test isolation.
        Self {
            inner: Box::leak(Box::new(Mutex::new(HashMap::new()))),
        }
    }
}

impl Kv for InProcessKv {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner
            .lock()
            .ok()
            .and_then(|m| m.get(key).cloned())
    }

    fn put(&self, key: &str, value: Vec<u8>) {
        if let Ok(mut m) = self.inner.lock() {
            m.insert(key.to_string(), value);
        }
    }
}

/// Host adapter: wraps the `BeeHostV1`
/// function pointers. For production:
/// the plugin's KV writes go to the
/// host's cluster KV.
pub struct HostKv {
    ctx: *mut std::ffi::c_void,
    host: *const crate::BeeHostV1,
}

// SAFETY: the host guarantees that the
// ctx + host pointers are `Send` (the
// FFI is single-threaded per plugin
// instance; the `Mutex` inside the host
// KV is not our concern). Plugin authors
// use `Arc<HostKv>` in the same thread
// they call `Kv::get` / `Kv::put`.
unsafe impl Send for HostKv {}
unsafe impl Sync for HostKv {}

impl HostKv {
    /// # Safety
    /// `host` must be a valid `BeeHostV1`
    /// pointer with `kv_get` and `kv_put`
    /// slots populated. `ctx` is the host's
    /// per-plugin context pointer (passed
    /// through to the FFI calls).
    pub unsafe fn new(
        host: *const crate::BeeHostV1,
        ctx: *mut std::ffi::c_void,
    ) -> Arc<Self> {
        Arc::new(Self { ctx, host })
    }
}

impl Kv for HostKv {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        unsafe {
            let host = &*self.host;
            let kv_get = host.kv_get?;
            let c_key = std::ffi::CString::new(key).ok()?;
            let mut out_value: *mut u8 = std::ptr::null_mut();
            let mut out_len: usize = 0;
            let rc = kv_get(
                self.ctx,
                c_key.as_ptr(),
                &mut out_value,
                &mut out_len,
            );
            match rc {
                0 => {
                    if out_value.is_null() || out_len == 0 {
                        return None;
                    }
                    let bytes =
                        std::slice::from_raw_parts(out_value, out_len)
                            .to_vec();
                    // The host-allocated bytes
                    // must be freed via the
                    // host's allocator. For the
                    // MVP the bytes are leaked
                    // (the plugin process exits
                    // shortly); a S33.6.x
                    // follow-up will thread the
                    // host's free fn pointer.
                    Some(bytes)
                }
                1 => None,
                _ => None,
            }
        }
    }

    fn put(&self, key: &str, value: Vec<u8>) {
        unsafe {
            let host = &*self.host;
            if let Some(kv_put) = host.kv_put {
                let c_key = match std::ffi::CString::new(key) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let _ = kv_put(
                    self.ctx,
                    c_key.as_ptr(),
                    value.as_ptr(),
                    value.len(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_process_kv_roundtrip() {
        let kv = InProcessKv::new();
        assert!(kv.get("k1").is_none());
        kv.put("k1", b"v1".to_vec());
        assert_eq!(kv.get("k1"), Some(b"v1".to_vec()));
    }

    #[test]
    fn in_process_kv_is_shared_across_adapters() {
        // Two InProcessKv::new() calls share
        // the same process-global map —
        // matching the prior kv_stub
        // semantics.
        let a = InProcessKv::new();
        let b = InProcessKv::new();
        a.put("shared", b"x".to_vec());
        assert_eq!(b.get("shared"), Some(b"x".to_vec()));
    }

    #[test]
    fn in_process_kv_default_is_isolated() {
        // Default::default() returns a
        // fresh per-instance KV (no
        // static), useful for per-test
        // isolation.
        let kv = InProcessKv::default();
        kv.put("k", b"v".to_vec());
        let other = InProcessKv::default();
        assert!(other.get("k").is_none());
    }

    #[test]
    fn host_kv_round_trip_through_mock_ffi() {
        // Mock KV store. The mock `kv_get` / `kv_put` functions
        // read/write this `Mutex<HashMap>` to simulate the host's
        // cluster KV.
        use std::collections::HashMap;
        use std::sync::Mutex;
        static STORE: std::sync::OnceLock<Mutex<HashMap<String, Vec<u8>>>> =
            std::sync::OnceLock::new();
        let store = STORE.get_or_init(|| Mutex::new(HashMap::new()));
        store.lock().unwrap().clear();

        // Mock FFI: kv_get reads from the store, kv_put writes to
        // the store. The closure must be `unsafe extern "C" fn` with
        // a C-compatible signature; it accesses the static STORE
        // directly (no captures).
        unsafe extern "C" fn mock_kv_get(
            _ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            out_value: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            let c_key = std::ffi::CStr::from_ptr(key);
            let key = c_key.to_str().unwrap();
            let store = STORE.get().unwrap();
            let store = store.lock().unwrap();
            match store.get(key) {
                Some(v) => {
                    let len = v.len();
                    let ptr = v.as_ptr();
                    *out_value = ptr as *mut u8;
                    *out_len = len;
                    0
                }
                None => 1,
            }
        }

        unsafe extern "C" fn mock_kv_put(
            _ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            value: *const u8,
            len: usize,
        ) -> i32 {
            let c_key = std::ffi::CStr::from_ptr(key);
            let key = c_key.to_str().unwrap().to_string();
            let bytes = std::slice::from_raw_parts(value, len).to_vec();
            let store = STORE.get().unwrap();
            store.lock().unwrap().insert(key, bytes);
            0
        }

        // Mock host with the FFI slots populated.
        let host = crate::BeeHostV1 {
            ctx: std::ptr::null_mut(),
            register_adapter: None,
            register_input_adapter_vtable: None,
            register_output_adapter_vtable: None,
            register_handler_vtable: None,
            kv_get: Some(mock_kv_get),
            kv_put: Some(mock_kv_put),
            kv_cas: None,
            secret_get: None,
            secret_put: None,
            current_stream_id: None,
        };

        let host_ptr = &host as *const crate::BeeHostV1;
        let kv = unsafe { HostKv::new(host_ptr, std::ptr::null_mut()) };

        // Round-trip: put then get.
        kv.put("hello", b"world".to_vec());
        assert_eq!(kv.get("hello"), Some(b"world".to_vec()));

        // Overwrite: put replaces.
        kv.put("hello", b"rust".to_vec());
        assert_eq!(kv.get("hello"), Some(b"rust".to_vec()));
    }

    #[test]
    fn host_kv_returns_none_on_not_found() {
        // Same mock FFI pattern as above; the test focuses on
        // the not-found return path.
        use std::collections::HashMap;
        use std::sync::Mutex;
        static STORE: std::sync::OnceLock<Mutex<HashMap<String, Vec<u8>>>> =
            std::sync::OnceLock::new();
        let store = STORE.get_or_init(|| Mutex::new(HashMap::new()));
        store.lock().unwrap().clear();

        unsafe extern "C" fn mock_kv_get(
            _ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            out_value: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            let c_key = std::ffi::CStr::from_ptr(key);
            let key = c_key.to_str().unwrap();
            let store = STORE.get().unwrap();
            let store = store.lock().unwrap();
            match store.get(key) {
                Some(v) => {
                    let len = v.len();
                    let ptr = v.as_ptr();
                    *out_value = ptr as *mut u8;
                    *out_len = len;
                    0
                }
                None => 1,
            }
        }

        let host = crate::BeeHostV1 {
            ctx: std::ptr::null_mut(),
            register_adapter: None,
            register_input_adapter_vtable: None,
            register_output_adapter_vtable: None,
            register_handler_vtable: None,
            kv_get: Some(mock_kv_get),
            kv_put: None,
            kv_cas: None,
            secret_get: None,
            secret_put: None,
            current_stream_id: None,
        };
        let host_ptr = &host as *const crate::BeeHostV1;
        let kv = unsafe { HostKv::new(host_ptr, std::ptr::null_mut()) };

        // No `put` ever called — get returns None.
        assert_eq!(kv.get("missing"), None);
    }
}
