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

/// Content-hash of the loaded plugin binary. Two builds of the same
/// logical plugin (even with the same version string) have distinct
/// `PluginId`s (ADR-0009). KV state keys include the hash so old
/// plugin state survives a swap.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginName(pub String);

impl std::fmt::Display for PluginName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Description of one Adapter provided by a Plugin. MVP: a name
/// plus an `is_input` flag. S19+ will add typed signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterDescriptor {
    pub name: String,
    pub is_input: bool,
}

/// Description of one Handler provided by a Plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerDescriptor {
    pub name: String,
}

/// Metadata a Plugin reports back to the host via `bee_plugin_init`.
///
/// The host (Bee) stores one `PluginManifest` per loaded `PluginId`.
/// The Compiler validates Pipeline definitions against the available
/// Plugins' manifests before submit.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

// `BeeHostV1` is a plain data struct; the host controls access
// through the `ctx` and ensures no data races. We don't derive
// Send/Sync because the struct contains a raw pointer, but the host
// never shares a `BeeHostV1` across threads.

/// Opaque handle the plugin owns. The plugin's `bee_plugin_init`
/// returns a `*mut PluginHandle` and `bee_plugin_drop` consumes it.
/// The host treats the pointer as opaque — it never dereferences it.
pub struct PluginHandle {
    /// The plugin's manifest (so the host can introspect without
    /// calling back into the plugin).
    pub manifest: PluginManifest,
    /// Optional plugin-private state. The Plugin SDK does not look
    /// inside; the host just keeps the `Arc` alive.
    pub inner: Arc<dyn std::any::Any + Send + Sync + 'static>,
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
}

pub type PluginResult<T> = std::result::Result<T, PluginError>;

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
}
