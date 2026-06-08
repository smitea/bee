//! S33 §3: registry of loaded-plugin vtables + trait wrappers that
//! implement the existing `InputAdapter` / `OutputAdapter` /
//! `Handler` traits by calling through the FFI vtables.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use bee_plugin_sdk::vtable::{HandlerVtable, InputAdapterVtable, OutputAdapterVtable};
use bee_adapter::{AdapterError, AdapterResult, Event, InputAdapter};
use bee_plugin_sdk::event::{decode_event, EventBytes};

/// Process-global registry of plugin vtables. Populated by the
/// host after `PluginManager::load_directory` registers each
/// plugin's `PluginHandle`. The runtime consults this when
/// starting a Phase to resolve an adapter name to a vtable.
pub struct PluginAdapterRegistry {
    input: RwLock<HashMap<String, *const InputAdapterVtable>>,
    output: RwLock<HashMap<String, *const OutputAdapterVtable>>,
    handler: RwLock<HashMap<String, *const HandlerVtable>>,
}

// SAFETY: The raw pointers stored inside the maps point to
// `#[repr(C)]` vtables that the host-side plugin shim owns (as a
// `static`). The registry itself never dereferences them — it
// stores and retrieves pointers verbatim. The actual dereference
// happens inside the trait wrappers below, in `unsafe` blocks that
// respect FFI safety rules. Sharing the registry across threads is
// sound because the underlying vtable `static`s are immutable.
unsafe impl Send for PluginAdapterRegistry {}
unsafe impl Sync for PluginAdapterRegistry {}

impl PluginAdapterRegistry {
    pub fn global() -> &'static Self {
        static REG: OnceLock<PluginAdapterRegistry> = OnceLock::new();
        REG.get_or_init(|| Self {
            input: RwLock::new(HashMap::new()),
            output: RwLock::new(HashMap::new()),
            handler: RwLock::new(HashMap::new()),
        })
    }

    pub fn register_input(&self, name: &str, vtable: *const InputAdapterVtable) {
        self.input.write().unwrap().insert(name.to_string(), vtable);
    }
    pub fn register_output(&self, name: &str, vtable: *const OutputAdapterVtable) {
        self.output.write().unwrap().insert(name.to_string(), vtable);
    }
    pub fn register_handler(&self, name: &str, vtable: *const HandlerVtable) {
        self.handler.write().unwrap().insert(name.to_string(), vtable);
    }

    pub fn lookup_input(&self, name: &str) -> Option<*const InputAdapterVtable> {
        self.input.read().unwrap().get(name).copied()
    }
    pub fn lookup_output(&self, name: &str) -> Option<*const OutputAdapterVtable> {
        self.output.read().unwrap().get(name).copied()
    }
    pub fn lookup_handler(&self, name: &str) -> Option<*const HandlerVtable> {
        self.handler.read().unwrap().get(name).copied()
    }
}

/// A plugin-backed InputAdapter that delegates to a loaded plugin's
/// `InputAdapterVtable`. The vtable is looked up by name at
/// construction time.
pub struct PluginInputAdapter {
    vtable: *const InputAdapterVtable,
    ctx: *mut std::ffi::c_void,
    /// Adapter name (e.g. "subscribe"); used for diagnostics.
    pub name: String,
}

// SAFETY: The `vtable` pointer points to a `#[repr(C)]` static
// vtable (immutable for the program's lifetime). The `ctx` pointer
// is the plugin's per-instance state — owned exclusively by this
// `PluginInputAdapter` (the trait methods all take `&mut self` or
// `self`, so only one thread accesses the ctx at a time). Sending
// the adapter to another thread is therefore sound.
unsafe impl Send for PluginInputAdapter {}

impl InputAdapter for PluginInputAdapter {
    type Config = Vec<u8>; // bincode-encoded plugin config

    async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let vtable = PluginAdapterRegistry::global()
            .lookup_input("default")
            .ok_or_else(|| AdapterError::Open("no plugin vtable registered for 'default'".into()))?;
        let ctx = unsafe {
            ((*vtable).open)(config.as_ptr(), config.len(), std::ptr::null_mut())
        };
        if ctx.is_null() {
            return Err(AdapterError::Open("plugin open returned null".into()));
        }
        Ok(Self { vtable, ctx, name: "default".into() })
    }

    async fn next(&mut self) -> AdapterResult<Option<Event>> {
        let mut out = EventBytes::EMPTY;
        let rc = unsafe { ((*self.vtable).next)(self.ctx, &mut out) };
        match rc {
            1 => {
                if out.ptr.is_null() || out.len == 0 {
                    return Ok(None);
                }
                let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
                let event = decode_event(bytes).map_err(|e| AdapterError::Next(e.to_string()))?;
                Ok(Some(event))
            }
            0 => Ok(None),
            _ => Err(AdapterError::Next("plugin returned -1".into())),
        }
    }

    async fn close(self) -> AdapterResult<()> {
        let rc = unsafe { ((*self.vtable).close)(self.ctx) };
        if rc != 0 {
            return Err(AdapterError::Close("plugin returned non-zero".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_plugin_sdk::vtable::InputAdapterVtable;
    use std::sync::Mutex;

    struct Ctx {
        event_bytes: Mutex<Vec<u8>>,
    }

    unsafe extern "C" fn fake_open(
        _config_ptr: *const u8,
        _config_len: usize,
        _err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void {
        let ctx = Box::new(Ctx { event_bytes: Mutex::new(vec![]) });
        Box::into_raw(ctx) as *mut std::ffi::c_void
    }

    unsafe extern "C" fn fake_next(
        ctx: *mut std::ffi::c_void,
        out: *mut EventBytes,
    ) -> i32 {
        let ctx = &*(ctx as *const Ctx);
        let bytes = ctx.event_bytes.lock().unwrap().clone();
        if bytes.is_empty() {
            *out = EventBytes::EMPTY;
            return 0;
        }
        let len = bytes.len();
        let ptr = bytes.as_ptr();
        std::mem::forget(bytes);
        *out = EventBytes { ptr, len };
        1
    }

    unsafe extern "C" fn fake_close(ctx: *mut std::ffi::c_void) -> i32 {
        if !ctx.is_null() {
            drop(Box::from_raw(ctx as *mut Ctx));
        }
        0
    }

    static FAKE_VTABLE: InputAdapterVtable = InputAdapterVtable {
        open: fake_open,
        next: fake_next,
        close: fake_close,
    };

    #[test]
    fn registry_lookup_returns_registered_vtable() {
        let reg = PluginAdapterRegistry::global();
        reg.register_input("s33-test-registered", &FAKE_VTABLE);
        let v = reg.lookup_input("s33-test-registered");
        assert!(v.is_some());
        reg.input.write().unwrap().remove("s33-test-registered");
    }

    #[test]
    fn registry_lookup_returns_none_for_unknown() {
        let reg = PluginAdapterRegistry::global();
        let v = reg.lookup_input("definitely-not-registered-anywhere-s33");
        assert!(v.is_none());
    }
}
