# 0005: Plugin FFI boundary — Rust cdylib for MVP, full C ABI in 1.x

The Q5 menu was C ABI / WASM / Rust trait objects / static link. We pick a path that fits MVP scope: **plugins are Rust crates compiled as `cdylib`**, exposed via `#[no_mangle] extern "C" fn bee_plugin_init(host: *mut BeeHost) -> *mut PluginHandle`, and loaded with `libloading::Library::new()`. The mechanism (dlopen, opaque handle, vtable) is the same as the future C ABI path; the *content* is Rust. This unlocks in-house and OSS-Rust plugin authors now, and leaves a clean migration path to full C ABI in 1.x — any C-compatible language (C / C++ / Python via ctypes / Go via cgo) can then write plugins against the same `BeeHost` C struct.

## Consequences

- **Plugin author must use the same Rust toolchain version as Bee** (Rust std ABI is not stable across versions). We will document the supported Rust version in the plugin SDK README and re-publish the SDK against every Bee release.
- The plugin SDK is a small Rust crate (`bee-plugin-sdk`) that defines the `Plugin` trait, the `BeeHostV1` C-ABI struct, and helper macros for declaring Adapters / Handlers. In-house plugin authors use it for ergonomics; future C-ABI plugin authors can ignore it and write against the raw `BeeHost` struct.
- Plugin lifecycle: Plugin Manager watches a configured directory (default `/etc/bee/plugins/`), loads new `.so` / `.dylib` / `.dll` files, registers their Adapters / Handlers with the local Registry, and unloads / reloads when files are removed or replaced (after reference count drops to zero).
- No WASM sandbox in MVP. If a user-uploaded ML model later needs sandboxing, that becomes a 1.x concern: run the plugin in a WASM subprocess exposing the same `BeeHost` interface, swap only the loader.
- Other-language plugin support (Python via ctypes, Go via cgo, etc.) is **explicitly deferred to 1.x**. MVP users who need a non-Rust SDK must wrap it in a Rust plugin that calls into the C ABI of the target SDK.
- Migration to C ABI in 1.x is mechanical: the `BeeHostV1` C struct is already in place. Adding new fields follows C-ABI versioning rules (append at the end, bump version number). Plugins that only use existing fields continue to work.
