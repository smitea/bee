//! `bee-plugin-sdk::macros` — S33: the `cdylib_plugin!` macro.
//!
//! Plugin authors use this macro to declare the FFI entry point
//! for a `cdylib` plugin. The macro generates two `#[no_mangle]
//! extern "C"` functions:
//!
//! - `bee_plugin_init(host) -> *mut PluginHandle`: the host calls
//!   this right after `dlopen` to obtain the plugin's
//!   `PluginHandle` (manifest + state).
//! - `bee_plugin_drop(handle)`: the host calls this when the
//!   `LoadedPlugin` is dropped (S19+ follow-up; the S33 path
//!   relies on the `Library` keeping the `.so` alive).
//!
//! ## Usage
//!
//! ```ignore
//! // In a plugin's lib.rs:
//! use bee_plugin_sdk::{cdylib_plugin, PluginHandle, PluginManifest, PluginResult};
//!
//! pub struct MyFactory;
//!
//! impl MyFactory {
//!     pub fn manifest() -> PluginManifest { ... }
//!     pub fn init() -> PluginResult<PluginHandle> { ... }
//! }
//!
//! cdylib_plugin!(MyFactory);
//! ```
//!
//! The macro requires the factory type to expose two associated
//! functions: `manifest()` and `init()`.

/// Generate the FFI entry symbols for a `cdylib` plugin.
///
/// The argument is the factory type (not an instance). The macro
/// generates code that constructs a `MyFactory` (the unit type)
/// and calls `<MyFactory as Factory>::manifest()` /
/// `<MyFactory as Factory>::init()`. The factory type must
/// implement the [`Factory`] trait.
///
/// See the module docs for the full usage pattern.
#[macro_export]
macro_rules! cdylib_plugin {
    ($factory:ty) => {
        $crate::cdylib_plugin_impl!($factory);
    };
}

/// Internal: the actual implementation. Exposed for the
/// `cdylib_plugin!` macro to call into. The plugin author
/// should not need to call this directly.
#[macro_export]
#[doc(hidden)]
macro_rules! cdylib_plugin_impl {
    ($factory:ty) => {
        /// FFI entry point: called by the host right after
        /// `dlopen`. Returns a `*mut PluginHandle` produced by
        /// `Arc::into_raw(Arc::new(handle))`. On error, returns
        /// null.
        ///
        /// The `_host` parameter is currently unused by S33 mock
        /// plugins; the S19+ follow-up will thread a real
        /// `BeeHostV1` through so plugins can register their
        /// Adapters / Handlers back into the host.
        #[no_mangle]
        pub extern "C" fn bee_plugin_init(
            _host: *mut $crate::BeeHostV1,
        ) -> *mut $crate::PluginHandle {
            let f = <$factory as $crate::Factory>::init();
            match f {
                Ok(handle) => ::std::sync::Arc::into_raw(
                    ::std::sync::Arc::new(handle),
                ) as *mut _,
                Err(_) => ::std::ptr::null_mut(),
            }
        }

        /// FFI cleanup: called by the host (S19+) when the
        /// `LoadedPlugin` is dropped. Recovers the `Arc` from
        /// the raw pointer and drops it. Safe to call with a
        /// null pointer (no-op).
        #[no_mangle]
        pub extern "C" fn bee_plugin_drop(
            handle: *mut $crate::PluginHandle,
        ) {
            if !handle.is_null() {
                unsafe {
                    drop(::std::sync::Arc::from_raw(handle));
                }
            }
        }
    };
}

/// Trait the `cdylib_plugin!` macro requires the factory type to
/// implement. Plugin authors implement this manually or via
/// the [`cdylib_factory!`] helper macro.
///
/// Two associated functions:
/// - [`Factory::manifest`]: builds the plugin's
///   [`PluginManifest`] (name, version, abi_version, adapters,
///   handlers).
/// - [`Factory::init`]: constructs the plugin's [`PluginHandle`]
///   (manifest + state).
///
/// The S33 pattern is:
/// ```ignore
/// pub struct MyFactory;
///
/// impl bee_plugin_sdk::Factory for MyFactory {
///     fn manifest() -> PluginManifest { ... }
///     fn init() -> PluginResult<PluginHandle> { ... }
/// }
/// ```
pub trait Factory {
    fn manifest() -> crate::PluginManifest;
    fn init() -> crate::PluginResult<crate::PluginHandle>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdapterDescriptor, Arc, PluginHandle, PluginManifest, PluginName};

    pub struct TestFactory;

    impl Factory for TestFactory {
        fn manifest() -> PluginManifest {
            PluginManifest {
                name: PluginName("test".into()),
                feature_version: "1.0.0".into(),
                abi_version: "v1".into(),
                adapters: vec![AdapterDescriptor {
                    name: "test_adapter".into(),
                    is_input: true,
                }],
                handlers: vec![],
            }
        }
        fn init() -> crate::PluginResult<PluginHandle> {
            Ok(PluginHandle {
                manifest: Self::manifest(),
                inner: Arc::new(()),
                input_adapters: std::collections::HashMap::new(),
                output_adapters: std::collections::HashMap::new(),
                handlers: std::collections::HashMap::new(),
            })
        }
    }

    // Generate the FFI entry symbols in this test scope.
    crate::cdylib_plugin!(TestFactory);

    #[test]
    fn factory_trait_returns_expected_manifest() {
        let m = TestFactory::manifest();
        assert_eq!(m.name.0, "test");
        assert_eq!(m.adapters.len(), 1);
    }

    #[test]
    fn factory_init_returns_valid_handle() {
        let h = TestFactory::init().unwrap();
        assert_eq!(h.manifest.name.0, "test");
    }

    #[test]
    fn cdylib_init_symbol_returns_non_null_pointer() {
        // Call the FFI entry point generated by `cdylib_plugin!`.
        // It must return a non-null pointer to a `PluginHandle`.
        let ptr = bee_plugin_init(std::ptr::null_mut());
        assert!(!ptr.is_null(), "init returned null");
        // Recover the Arc to test the round-trip.
        let arc = unsafe { Arc::from_raw(ptr) };
        assert_eq!(arc.manifest.name.0, "test");
    }

    #[test]
    fn cdylib_drop_symbol_handles_null() {
        // Null pointer is a no-op.
        bee_plugin_drop(std::ptr::null_mut());
    }
}

/// S33.6: register a sequence of plugin
/// vtables into the 3 `HashMap` fields of a
/// `PluginHandle` (input_adapters /
/// output_adapters / handlers).
///
/// Usage:
/// ```ignore
/// let mut input_adapters = HashMap::new();
/// let mut output_adapters = HashMap::new();
/// let mut handlers = HashMap::new();
/// register_vtable! {
///     input_adapters, output_adapters, handlers;
///     input  "subscribe" => &SUBSCRIBE_VTABLE,
///     output "ohlcv"     => &OHLCV_VTABLE,
///     handler "fib"      => &FIB_VTABLE,
/// }
/// ```
#[macro_export]
macro_rules! register_vtable {
    (
        $input:ident, $output:ident, $handlers:ident;
        $( $kind:ident $name:literal => $vtable:expr ),* $(,)?
    ) => {
        $(
            $crate::register_vtable!(@branch $kind, $input, $output, $handlers, $name, $vtable);
        )*
    };
    (@branch input, $input:ident, $output:ident, $handlers:ident, $name:literal, $vtable:expr) => {
        $input.insert(
            ::std::string::String::from($name),
            $vtable as *const ::bee_plugin_sdk::vtable::InputAdapterVtable,
        );
    };
    (@branch output, $input:ident, $output:ident, $handlers:ident, $name:literal, $vtable:expr) => {
        $output.insert(
            ::std::string::String::from($name),
            $vtable as *const ::bee_plugin_sdk::vtable::OutputAdapterVtable,
        );
    };
    (@branch handler, $input:ident, $output:ident, $handlers:ident, $name:literal, $vtable:expr) => {
        $handlers.insert(
            ::std::string::String::from($name),
            $vtable as *const ::bee_plugin_sdk::vtable::HandlerVtable,
        );
    };
}
