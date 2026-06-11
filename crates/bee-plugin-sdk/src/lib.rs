//! `bee-plugin-sdk` — Bee Plugin contract (S19, ADR-0005 / ADR-0009).
//!
//! A Bee Plugin is a dynamically loaded Rust `cdylib` that implements
//! one or more Adapters or Handlers. The host (Bee) loads the binary,
//! hashes its content to derive a `PluginId`, and calls the plugin's
//! `bee_plugin_init` to obtain a `*mut PluginHandle`. From that point
//! on, the plugin calls back into the host through the [`BeeHostV1`]
//! C struct (function pointer table) to register its Adapters /
//! Handlers with the local Registry.
//!
//! ## S19 MVP scope
//! - `PluginId` (content-hash) and the `compute_plugin_id` helper
//! - `PluginManifest` (the metadata the plugin reports back to the host)
//! - `BeeHostV1` C struct (placeholder FFI table — the function
//!   pointers are typed `Option<fn(...)>` so the plugin sees a real
//!   function pointer signature; the host wires up the table)
//! - `PluginHandle` opaque type (plugin-side state)
//! - `Plugin` trait (the Rust trait a `cdylib` plugin implements)
//!
//! Out of S19 scope (follow-ups):
//! - `libloading` glue in `bee-registry` (S19+ follow-up)
//! - A test `cdylib` plugin that exercises real FFI (S19+ follow-up)
//! - Multi-version coexistence (S21)
//! - ABI version check (S20)

use std::sync::Arc;

use sha2::{Digest, Sha256};

pub mod event;
pub mod macros;
pub mod vtable;
pub use macros::Factory;

/// Content-hash of the loaded plugin binary. Two builds of the same
/// logical plugin (even with the same version string) have distinct
/// `PluginId`s (ADR-0009). KV state keys include the hash so old
/// plugin state survives a swap.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PluginId(pub String);

impl std::fmt::Display for PluginId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl PluginId {
    /// Length of the hex-encoded sha256 digest (64 chars).
    pub const HEX_LEN: usize = 64;

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Compute the [`PluginId`] for a given plugin binary content. The
/// hash is over the raw bytes of the `.so`/`.dylib`/`.dll` file (or
/// the in-memory bytes of a `cdylib` for tests). Same content →
/// same `PluginId`; one byte different → different `PluginId`.
pub fn compute_plugin_id(content: &[u8]) -> PluginId {
    let mut hasher = Sha256::new();
    hasher.update(content);
    let digest = hasher.finalize();
    PluginId(hex::encode(digest))
}

/// Logical name of a Plugin (e.g. "binance"). Author-chosen, not
/// guaranteed unique — the binding truth is the [`PluginId`] hash.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PluginName(pub String);

impl std::fmt::Display for PluginName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Description of one Adapter provided by a Plugin. MVP: a name
/// plus an `is_input` flag. S19+ will add typed signatures.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AdapterDescriptor {
    pub name: String,
    pub is_input: bool,
}

/// Description of one Handler provided by a Plugin.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandlerDescriptor {
    pub name: String,
}

/// Metadata a Plugin reports back to the host via `bee_plugin_init`.
///
/// The host (Bee) stores one `PluginManifest` per loaded `PluginId`.
/// The Compiler validates Pipeline definitions against the available
/// Plugins' manifests before submit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PluginManifest {
    /// Logical name (e.g. "binance"). Human-friendly, not unique.
    pub name: PluginName,
    /// Author-chosen feature version (e.g. "1.4.2"). Human-readable;
    /// not the binding truth — [`PluginId`] is.
    pub feature_version: String,
    /// ABI version this Plugin was compiled against (e.g. "v1"). The
    /// host checks this at load time (S20).
    pub abi_version: String,
    /// Adapters the Plugin provides.
    pub adapters: Vec<AdapterDescriptor>,
    /// Handlers the Plugin provides.
    pub handlers: Vec<HandlerDescriptor>,
}

/// The host API the plugin sees (a function pointer table). MVP:
/// one method, `register_adapter`, for the plugin to push Adapter
/// descriptors onto the host's registry. S19+ extends with handler
/// registration, secret lookup, and observability hooks.
///
/// ## FFI safety
/// `BeeHostV1` is `#[repr(C)]` so the layout is stable across the
/// FFI boundary. The plugin receives a `*mut BeeHostV1` and calls
/// through the function pointers. The host owns the underlying
/// `BeeHostV1Inner` and keeps it alive for the plugin's lifetime
/// (freed by the plugin's `bee_plugin_drop` symbol).
#[repr(C)]
pub struct BeeHostV1 {
    /// Opaque handle the host allocates; the plugin stores this and
    /// passes it back to every host call so the host can recover its
    /// context.
    pub ctx: *mut std::ffi::c_void,
    /// Register an Adapter descriptor on the host. The descriptor's
    /// `name` and `is_input` flag let the Compiler/Registry match SQL
    /// references (e.g. `binance.subscribe(...)`) to the Plugin.
    pub register_adapter:
        Option<unsafe extern "C" fn(ctx: *mut std::ffi::c_void, adapter: *const AdapterDescriptor)>,

    // S33-deferred: register the plugin's vtable alongside the
    // adapter descriptor. The host stores the vtable pointer in
    // its PluginHandle; the runtime consults it on every event.
    pub register_input_adapter_vtable:
        Option<unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
            vtable: *const vtable::InputAdapterVtable,
        )>,
    pub register_output_adapter_vtable:
        Option<unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
            vtable: *const vtable::OutputAdapterVtable,
        )>,
    pub register_handler_vtable:
        Option<unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
            vtable: *const vtable::HandlerVtable,
        )>,

    /// KV: read a value by key. On success, returns 0 and writes
    /// the value pointer + length to `out_value` and `out_len` (caller
    /// frees via `host_alloc_free`). On not-found, returns 1. On error,
    /// returns -1.
    pub kv_get: Option<
        unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            out_value: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// KV: write a value (overwrites). Returns 0 on success, -1 on error.
    pub kv_put: Option<
        unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            value: *const u8,
            len: usize,
        ) -> i32,
    >,

    /// KV: compare-and-swap. Returns 0 on success, 1 on mismatch, -1 on error.
    pub kv_cas: Option<
        unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            expected: *const u8,
            exp_len: usize,
            new: *const u8,
            new_len: usize,
        ) -> i32,
    >,

    /// Get the current stream_id (32-byte hash of the SQL call site).
    /// Returns 0 on success, -1 on error.
    pub current_stream_id: Option<
        unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            out_id: *mut [u8; 32],
        ) -> i32,
    >,
}

// `BeeHostV1` is a plain data struct; the host controls access
// through the `ctx` and ensures no data races. We don't derive
// Send/Sync because the struct contains a raw pointer, but the host
// never shares a `BeeHostV1` across threads.

impl BeeHostV1 {
    /// Safe wrapper for `kv_get`. Returns `Ok(Some(value))` if found,
    /// `Ok(None)` if not found, `Err(SdkError)` on error.
    pub fn safe_kv_get(&self, key: &str) -> Result<Option<Vec<u8>>, SdkError> {
        let kv_get = self
            .kv_get
            .ok_or(SdkError::HostFnMissing("kv_get"))?;
        let c_key = std::ffi::CString::new(key)
            .map_err(|_| SdkError::InvalidKey(key.into()))?;
        let mut out_value: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let rc = unsafe {
            kv_get(self.ctx, c_key.as_ptr(), &mut out_value, &mut out_len)
        };
        match rc {
            0 => {
                if out_len == 0 {
                    return Ok(Some(Vec::new()));
                }
                let value =
                    unsafe { std::slice::from_raw_parts(out_value, out_len) }
                        .to_vec();
                Ok(Some(value))
            }
            1 => Ok(None),
            _ => Err(SdkError::KvError("kv_get failed")),
        }
    }

    /// Safe wrapper for `kv_put`. Writes (or overwrites) `value` at
    /// `key`. Returns `Ok(())` on success, `Err(SdkError)` on error.
    pub fn safe_kv_put(&self, key: &str, value: &[u8]) -> Result<(), SdkError> {
        let kv_put = self
            .kv_put
            .ok_or(SdkError::HostFnMissing("kv_put"))?;
        let c_key = std::ffi::CString::new(key)
            .map_err(|_| SdkError::InvalidKey(key.into()))?;
        let rc = unsafe {
            kv_put(self.ctx, c_key.as_ptr(), value.as_ptr(), value.len())
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(SdkError::KvError("kv_put failed"))
        }
    }

    /// Safe wrapper for `kv_cas`. Compares `expected` to the current
    /// value at `key`; on match, writes `new` and returns `Ok(true)`.
    /// On mismatch, returns `Ok(false)`. On error, returns
    /// `Err(SdkError)`.
    pub fn safe_kv_cas(
        &self,
        key: &str,
        expected: &[u8],
        new: &[u8],
    ) -> Result<bool, SdkError> {
        let kv_cas = self
            .kv_cas
            .ok_or(SdkError::HostFnMissing("kv_cas"))?;
        let c_key = std::ffi::CString::new(key)
            .map_err(|_| SdkError::InvalidKey(key.into()))?;
        let rc = unsafe {
            kv_cas(
                self.ctx,
                c_key.as_ptr(),
                expected.as_ptr(),
                expected.len(),
                new.as_ptr(),
                new.len(),
            )
        };
        match rc {
            0 => Ok(true),
            1 => Ok(false),
            _ => Err(SdkError::KvError("kv_cas failed")),
        }
    }

    /// Safe wrapper for `current_stream_id`. Returns the 32-byte
    /// stream_id hash for the current SQL call site.
    pub fn safe_current_stream_id(&self) -> Result<[u8; 32], SdkError> {
        let f = self
            .current_stream_id
            .ok_or(SdkError::HostFnMissing("current_stream_id"))?;
        let mut out_id = [0u8; 32];
        let rc = unsafe { f(self.ctx, &mut out_id) };
        if rc == 0 {
            Ok(out_id)
        } else {
            Err(SdkError::KvError("current_stream_id failed"))
        }
    }
}

/// Opaque handle the plugin owns. The plugin's `bee_plugin_init`
/// returns a `*mut PluginHandle` and `bee_plugin_drop` consumes it.
/// The host treats the pointer as opaque — it never dereferences it.
// SAFETY: The raw `*const Vtable` pointers in `input_adapters`,
// `output_adapters`, and `handlers` point to `#[repr(C)]` vtable
// `static`s owned by the plugin's cdylib. The vtables are
// immutable for the plugin's lifetime, and the host never
// mutates them. Sharing the handle across threads is sound
// because (a) the `inner: Arc<dyn Any + Send + Sync>` is already
// Sync, and (b) the vtable pointers are only read (via FFI
// call) under controlled unsafe blocks.
unsafe impl Send for PluginHandle {}
unsafe impl Sync for PluginHandle {}

pub struct PluginHandle {
    /// The plugin's manifest (so the host can introspect without
    /// calling back into the plugin).
    pub manifest: PluginManifest,
    /// Optional plugin-private state. The Plugin SDK does not look
    /// inside; the host just keeps the `Arc` alive.
    pub inner: Arc<dyn std::any::Any + Send + Sync + 'static>,

    // S33-deferred: per-adapter vtable registries. Populated by
    // the plugin in `init()` and frozen for the plugin's
    // lifetime. The host looks up vtables by adapter/handler
    // name and calls through the function pointers.
    pub input_adapters:
        std::collections::HashMap<String, *const vtable::InputAdapterVtable>,
    pub output_adapters:
        std::collections::HashMap<String, *const vtable::OutputAdapterVtable>,
    pub handlers:
        std::collections::HashMap<String, *const vtable::HandlerVtable>,
}

/// Rust trait a plugin's `cdylib` crate implements. The plugin's
/// `bee_plugin_init` is a thin wrapper that constructs the
/// `PluginHandle` and returns it as an opaque pointer.
///
/// For the in-process test path (S19 MVP), Bee can directly call
/// `init` and keep the returned `PluginHandle` in the
/// `PluginManager`. For the real FFI path (S19+ follow-up), the
/// `cdylib`'s `bee_plugin_init` is loaded via `libloading` and the
/// result is wrapped into a `PluginHandle` on the host side.
pub trait Plugin: Send + Sync + 'static {
    /// The plugin's identity (used to compute the `PluginId`).
    /// MVP: derived from a static byte slice (e.g. the contents of
    /// a built-in mock plugin). Real plugins will use the loaded
    /// binary's bytes.
    fn plugin_content(&self) -> &'static [u8];

    /// Build the [`PluginManifest`].
    fn manifest(&self) -> PluginManifest;

    /// Initialize the plugin and return its handle.
    fn init(&self) -> PluginResult<PluginHandle>;

    /// Drop the plugin. The default impl just drops the inner state.
    fn drop(self) where Self: Sized {}
}

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("init: {0}")]
    Init(String),
    #[error("register: {0}")]
    Register(String),
    #[error("not found: {0}")]
    NotFound(String),
    /// S20: plugin's declared `abi_version` is not in the host's
    /// supported major-version set. The `.so`/`.dylib`/`.dll` is NOT
    /// deleted — it stays on disk for inspection.
    #[error(
        "plugin load rejected: hash={hash} claimed_abi={claimed} \
         expected_majors={expected:?} (see {migration_link})"
    )]
    AbiMismatch {
        hash: String,
        claimed: String,
        expected: Vec<u32>,
        migration_link: String,
    },
    /// S20: `abi_version` string in the plugin's [`PluginManifest`]
    /// could not be parsed (e.g. "garbage" or "").
    #[error("invalid abi_version '{0}' (expected form like '1.0' or 'v1')")]
    InvalidAbiVersion(String),
    /// S21: `feature_version` or `VersionSpec` string could not be
    /// parsed into a [`Version`] (e.g. "1.0.3.4" or "abc").
    #[error("invalid version '{0}' (expected form like '1.0.0' or 'v1.4.2')")]
    InvalidVersion(String),
}

pub type PluginResult<T> = std::result::Result<T, PluginError>;

/// Errors returned by the safe Rust wrappers around the [`BeeHostV1`]
/// function pointers. The host may be missing a function pointer
/// (older ABI), the plugin may have passed an invalid key (e.g. an
/// interior NUL), or the underlying KV call may have failed.
#[derive(Debug)]
pub enum SdkError {
    /// The host's [`BeeHostV1`] table did not have this function
    /// pointer set (e.g. an older host ABI).
    HostFnMissing(&'static str),
    /// The key contained an interior NUL byte and could not be
    /// converted to a C string.
    InvalidKey(String),
    /// The underlying host KV call returned a non-success code that
    /// is not part of the documented success/not-found contract.
    KvError(&'static str),
}

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SdkError::HostFnMissing(name) => {
                write!(f, "host function pointer {} is None", name)
            }
            SdkError::InvalidKey(k) => write!(f, "invalid KV key: {}", k),
            SdkError::KvError(msg) => write!(f, "KV error: {}", msg),
        }
    }
}

impl std::error::Error for SdkError {}

/// Link to the ABI migration guide, included in the
/// `PluginError::AbiMismatch` message. The MVP placeholder is a
/// stable URL that the user can find via the docs site search; a
/// real Bee release wires this to a versioned docs page.
pub const MIGRATION_DOC_LINK: &str =
    "https://github.com/bee/bee/blob/main/docs/adr/0009-plugin-multiversion-hash-abi.md";

/// Parsed form of a plugin's `abi_version` string. MVP: `(major, minor)`
/// parsed from `"1.0"`, `"v1"`, `"v1.5"`, etc. The host's expected
/// range is a list of accepted major versions (e.g. `[1]` for
/// `"1.x"`, `[1, 2]` for `"1.x"` or `"2.x"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AbiVersion {
    pub major: u32,
    pub minor: u32,
}

impl std::fmt::Display for AbiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl AbiVersion {
    /// Parse `"1.0"`, `"v1"`, `"v1.5"`, etc. The leading `v` is
    /// optional. A missing minor defaults to `0`.
    pub fn parse(s: &str) -> Result<Self, PluginError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(PluginError::InvalidAbiVersion(s.to_string()));
        }
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut parts = s.split('.');
        let major_str = parts.next().unwrap_or("");
        let major: u32 = major_str.parse().map_err(|_| {
            PluginError::InvalidAbiVersion(s.to_string())
        })?;
        let minor: u32 = parts
            .next()
            .map(|p| {
                p.parse()
                    .map_err(|_| PluginError::InvalidAbiVersion(s.to_string()))
            })
            .transpose()?
            .unwrap_or(0);
        // Anything after the minor (e.g. "1.0.3") is rejected for
        // MVP — abi_version is intentionally simple.
        if parts.next().is_some() {
            return Err(PluginError::InvalidAbiVersion(s.to_string()));
        }
        Ok(Self { major, minor })
    }

    /// True if this version's major is in the host's accepted list.
    pub fn matches_major(&self, accepted_majors: &[u32]) -> bool {
        accepted_majors.contains(&self.major)
    }
}

/// SemVer-style version (`major.minor.patch`). Distinct from
/// [`AbiVersion`] which is the *binary* contract version (per
/// ADR-0009, the major number is what the host accepts). Feature
/// version follows SemVer and is used by [`VersionSpec`] for
/// `binance:^1.0` / `binance:latest` style references in Pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Version {
    /// Parse `"1"`, `"1.0"`, `"1.4.2"`, `"v2.0"`. Missing parts
    /// default to `0`. A leading `v` is optional.
    pub fn parse(s: &str) -> Result<Self, PluginError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(PluginError::InvalidVersion(s.to_string()));
        }
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut parts = s.split('.');
        let parse_part = |part: &str| -> Result<u32, PluginError> {
            if part.is_empty() {
                Err(PluginError::InvalidVersion(s.to_string()))
            } else {
                part.parse::<u32>()
                    .map_err(|_| PluginError::InvalidVersion(s.to_string()))
            }
        };
        let major = parse_part(parts.next().unwrap_or("0"))?;
        let minor = parts
            .next()
            .map(parse_part)
            .transpose()?
            .unwrap_or(0);
        let patch = parts
            .next()
            .map(parse_part)
            .transpose()?
            .unwrap_or(0);
        if parts.next().is_some() {
            return Err(PluginError::InvalidVersion(s.to_string()));
        }
        Ok(Self { major, minor, patch })
    }
}

/// SemVer range syntax for Plugin references in Pipelines
/// (`binance:^1.0`, `binance:~1.2`, `binance:1.4.2`, `binance:latest`).
///
/// - [`VersionSpec::Exact`]: matches only the exact Version.
/// - [`VersionSpec::Compatible`]: `^1.0` → `>=1.0.0, <2.0.0`.
/// - [`VersionSpec::Patch`]: `~1.2` → `>=1.2.0, <1.3.0`.
/// - [`VersionSpec::Latest`]: any version; the resolver picks the
///   highest one loaded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VersionSpec {
    Exact(Version),
    Compatible(Version),
    Patch(Version),
    Latest,
}

impl std::fmt::Display for VersionSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionSpec::Exact(v) => write!(f, "{v}"),
            VersionSpec::Compatible(v) => write!(f, "^{v}"),
            VersionSpec::Patch(v) => write!(f, "~{v}"),
            VersionSpec::Latest => f.write_str("latest"),
        }
    }
}

impl VersionSpec {
    /// Parse `"1.4.2"` (exact), `"^1.0"` (compatible), `"~1.2"`
    /// (patch), `"latest"`. Other forms (compound ranges like
    /// `>=1.0,<2.0`) are deferred to 1.x per the S21 spec scope.
    pub fn parse(s: &str) -> Result<Self, PluginError> {
        let s = s.trim();
        if s == "latest" {
            return Ok(VersionSpec::Latest);
        }
        if let Some(rest) = s.strip_prefix('^') {
            return Ok(VersionSpec::Compatible(Version::parse(rest)?));
        }
        if let Some(rest) = s.strip_prefix('~') {
            return Ok(VersionSpec::Patch(Version::parse(rest)?));
        }
        Ok(VersionSpec::Exact(Version::parse(s)?))
    }

    /// True if `v` satisfies this spec.
    pub fn matches(&self, v: &Version) -> bool {
        match self {
            VersionSpec::Exact(target) => v == target,
            VersionSpec::Compatible(min) => {
                v >= min && v.major == min.major
            }
            VersionSpec::Patch(min) => {
                v >= min && v.major == min.major && v.minor == min.minor
            }
            VersionSpec::Latest => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_is_deterministic_sha256_hex() {
        let content = b"hello world";
        let id = compute_plugin_id(content);
        assert_eq!(id.0.len(), PluginId::HEX_LEN);
        assert_eq!(id, compute_plugin_id(content));
    }

    #[test]
    fn plugin_id_changes_with_content() {
        let a = compute_plugin_id(b"hello");
        let b = compute_plugin_id(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn manifest_is_clone_and_eq() {
        let m = PluginManifest {
            name: PluginName("binance".into()),
            feature_version: "1.0.0".into(),
            abi_version: "v1".into(),
            adapters: vec![AdapterDescriptor {
                name: "subscribe".into(),
                is_input: true,
            }],
            handlers: vec![],
        };
        assert_eq!(m, m.clone());
    }

    #[test]
    fn bee_host_v1_is_ffi_safe() {
        // Compile-time check that the struct has a stable layout.
        // (Smoke test: the size is non-zero, the field offsets are
        // predictable for the C side.)
        assert!(std::mem::size_of::<BeeHostV1>() > 0);
    }

    #[test]
    fn bee_host_v1_has_register_vtable_slots() {
        // Compile-time check: the new slots are present.
        fn _check_slots(h: &BeeHostV1) {
            let _ = h.register_input_adapter_vtable;
            let _ = h.register_output_adapter_vtable;
            let _ = h.register_handler_vtable;
        }
    }

    // ---- BeeHostV1 KV extension (S41 Task 2) ----

    #[test]
    fn bee_host_v1_has_kv_function_pointers() {
        // Compile-time check: the 4 new function-pointer fields exist on
        // BeeHostV1 and are of the right type. The test value is irrelevant
        // — the field types and the field set are what we care about.
        let host = BeeHostV1 {
            ctx: std::ptr::null_mut(),
            register_adapter: None,
            register_input_adapter_vtable: None,
            register_output_adapter_vtable: None,
            register_handler_vtable: None,
            kv_get: None,
            kv_put: None,
            kv_cas: None,
            current_stream_id: None,
        };
        assert!(host.kv_get.is_none());
        assert!(host.kv_put.is_none());
        assert!(host.kv_cas.is_none());
        assert!(host.current_stream_id.is_none());
    }

    // ---- AbiVersion (S20) ----

    #[test]
    fn abi_version_parses_basic_forms() {
        assert_eq!(AbiVersion::parse("1.0").unwrap(), AbiVersion { major: 1, minor: 0 });
        assert_eq!(AbiVersion::parse("1.5").unwrap(), AbiVersion { major: 1, minor: 5 });
        assert_eq!(AbiVersion::parse("v1").unwrap(), AbiVersion { major: 1, minor: 0 });
        assert_eq!(AbiVersion::parse("v2.3").unwrap(), AbiVersion { major: 2, minor: 3 });
        assert_eq!(AbiVersion::parse("10").unwrap(), AbiVersion { major: 10, minor: 0 });
    }

    #[test]
    fn abi_version_rejects_garbage() {
        assert!(matches!(AbiVersion::parse(""), Err(PluginError::InvalidAbiVersion(_))));
        assert!(matches!(AbiVersion::parse("garbage"), Err(PluginError::InvalidAbiVersion(_))));
        assert!(matches!(AbiVersion::parse("1.0.3"), Err(PluginError::InvalidAbiVersion(_))));
    }

    #[test]
    fn abi_version_matches_major_list() {
        let v1 = AbiVersion::parse("1.5").unwrap();
        let v2 = AbiVersion::parse("2.0").unwrap();
        assert!(v1.matches_major(&[1]));
        assert!(!v1.matches_major(&[2]));
        assert!(v2.matches_major(&[1, 2]));
        assert!(!v2.matches_major(&[]));
    }

    // ---- Version / VersionSpec (S21) ----

    #[test]
    fn version_parses_basic_forms() {
        assert_eq!(
            Version::parse("1").unwrap(),
            Version { major: 1, minor: 0, patch: 0 }
        );
        assert_eq!(
            Version::parse("1.4").unwrap(),
            Version { major: 1, minor: 4, patch: 0 }
        );
        assert_eq!(
            Version::parse("1.4.2").unwrap(),
            Version { major: 1, minor: 4, patch: 2 }
        );
        assert_eq!(
            Version::parse("v2.0.0").unwrap(),
            Version { major: 2, minor: 0, patch: 0 }
        );
    }

    #[test]
    fn version_rejects_garbage() {
        assert!(matches!(Version::parse(""), Err(PluginError::InvalidVersion(_))));
        assert!(matches!(Version::parse("abc"), Err(PluginError::InvalidVersion(_))));
        assert!(matches!(Version::parse("1.2.3.4"), Err(PluginError::InvalidVersion(_))));
    }

    #[test]
    fn version_ordering_is_lexicographic_semver() {
        let v100 = Version::parse("1.0.0").unwrap();
        let v110 = Version::parse("1.1.0").unwrap();
        let v200 = Version::parse("2.0.0").unwrap();
        let v101 = Version::parse("1.0.1").unwrap();
        assert!(v100 < v101);
        assert!(v101 < v110);
        assert!(v110 < v200);
    }

    #[test]
    fn version_spec_parses_all_four_forms() {
        assert_eq!(
            VersionSpec::parse("1.4.2").unwrap(),
            VersionSpec::Exact(Version { major: 1, minor: 4, patch: 2 })
        );
        assert_eq!(
            VersionSpec::parse("^1.0").unwrap(),
            VersionSpec::Compatible(Version { major: 1, minor: 0, patch: 0 })
        );
        assert_eq!(
            VersionSpec::parse("~1.2").unwrap(),
            VersionSpec::Patch(Version { major: 1, minor: 2, patch: 0 })
        );
        assert_eq!(VersionSpec::parse("latest").unwrap(), VersionSpec::Latest);
    }

    #[test]
    fn version_spec_matches_semver_semantics() {
        let v142 = Version::parse("1.4.2").unwrap();
        let v100 = Version::parse("1.0.0").unwrap();
        let v110 = Version::parse("1.1.0").unwrap();
        let v200 = Version::parse("2.0.0").unwrap();
        let v120 = Version::parse("1.2.0").unwrap();
        let v121 = Version::parse("1.2.1").unwrap();
        let v130 = Version::parse("1.3.0").unwrap();

        // Exact
        assert!(VersionSpec::Exact(v142).matches(&v142));
        assert!(!VersionSpec::Exact(v142).matches(&v100));

        // Compatible: ^1.0 → 1.0.0 <= v < 2.0.0
        assert!(VersionSpec::Compatible(v100).matches(&v100));
        assert!(VersionSpec::Compatible(v100).matches(&v142));
        assert!(VersionSpec::Compatible(v100).matches(&v110));
        assert!(!VersionSpec::Compatible(v100).matches(&v200));

        // Patch: ~1.2 → 1.2.0 <= v < 1.3.0
        assert!(VersionSpec::Patch(v120).matches(&v120));
        assert!(VersionSpec::Patch(v120).matches(&v121));
        assert!(!VersionSpec::Patch(v120).matches(&v130));
        assert!(!VersionSpec::Patch(v120).matches(&v110));
        assert!(!VersionSpec::Patch(v120).matches(&v200));

        // Latest: always true
        assert!(VersionSpec::Latest.matches(&v100));
        assert!(VersionSpec::Latest.matches(&v200));
    }
}
