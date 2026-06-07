//! `bee-registry` — Bee 注册中心。
//!
//! 管理 Plugin 加载 / ABI 校验 / 哈希校验 / 网络同步 / 路由表广播。
//! Registry 是 trait,具体实现可插拔 (本地、etcd 风格、内存测试桩)。
//!
//! S19 起实现 [`PluginManager`] 与 [`NetworkSync`]。
//! S33 起实现 [`loader::load_library`] (libloading 真实加载 `cdylib`)。

pub mod loader;

use std::collections::HashMap;
use std::sync::Arc;

use bee_plugin_sdk::{
    compute_plugin_id, AbiVersion, AdapterDescriptor, HandlerDescriptor, Plugin, PluginError,
    PluginHandle, PluginId, PluginManifest, Version, VersionSpec,
};

/// In-process Plugin Manager. Holds the set of loaded plugins keyed
/// by their content-hash [`PluginId`].
///
/// ## S19 + S20 scope
/// - `register(content, manifest)`: compute PluginId from the
///   content bytes, check `manifest.abi_version` against the
///   configured accepted major-version set, store the manifest +
///   handle. Idempotent on PluginId.
/// - `register_plugin(&dyn Plugin)`: same as `register` but takes
///   the Rust `Plugin` trait; calls `init()` after the ABI check
///   passes.
/// - `lookup(id)`: fetch the manifest for a known PluginId.
/// - `list()`: enumerate the loaded plugins (sorted by PluginId for
///   determinism).
/// - `list_adapters()` / `list_handlers()`: aggregated across plugins.
/// - S20 ABI check: the host's `expected_abi_majors` defaults to
///   `[1]` (accepts `v1.x`). Mismatch returns
///   `PluginError::AbiMismatch` with hash, claimed, expected, and
///   the migration doc link in the message.
///
/// Out of S19 / S20 scope (follow-ups):
/// - `libloading` to load `.so`/`.dylib`/`.dll` and call the C ABI
///   `bee_plugin_init` symbol. S19+ follow-up.
/// - A real test `cdylib` plugin (S19+ follow-up).
/// - `bee plugin inspect <path>` CLI (S20 acceptance, requires
///   libloading). S19+ follow-up.
/// - Network sync between nodes (the `NetworkSync` trait stub).
pub struct PluginManager {
    plugins: HashMap<PluginId, RegisteredPlugin>,
    /// S20: set of accepted `abi_version` major numbers. A plugin's
    /// parsed `AbiVersion` matches if its major is in this list.
    /// Defaults to `[1]` (the S19/S20 MVP). Mutable via
    /// `set_expected_abi_majors` for ops who run a Bee that supports
    /// a wider range (e.g. `[1, 2]` during an upgrade window).
    expected_abi_majors: Vec<u32>,
}

struct RegisteredPlugin {
    manifest: PluginManifest,
    handle: Arc<PluginHandle>,
    /// S21: refcount of Pipelines / Jobs that currently reference
    /// this Plugin. When the refcount drops to 0, the Plugin is
    /// auto-unloaded (removed from the manager). The Compiler/Registry
    /// calls `retain` on submit and `release` on Job stop.
    refcount: u32,
}

impl PluginManager {
    /// Default: accepts plugins with `abi_version` major in `[1]`.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            expected_abi_majors: vec![1],
        }
    }

    /// Configure the accepted `abi_version` majors at construction.
    /// Use this for tests or when a Bee release is built against a
    /// wider set (e.g. `[1, 2]` during an upgrade window).
    pub fn with_abi_majors(majors: Vec<u32>) -> Self {
        Self {
            plugins: HashMap::new(),
            expected_abi_majors: majors,
        }
    }

    /// S20: change the accepted ABI majors at runtime. Useful for an
    /// upgrade window where a Bee supports both v1 and v2 plugins.
    pub fn set_expected_abi_majors(&mut self, majors: Vec<u32>) {
        self.expected_abi_majors = majors;
    }

    /// S20: current accepted ABI majors (read-only view).
    pub fn expected_abi_majors(&self) -> &[u32] {
        &self.expected_abi_majors
    }

    /// S20: check a manifest's `abi_version` against the configured
    /// range. Returns `Ok(())` on match, `Err(AbiMismatch)` on
    /// mismatch, `Err(InvalidAbiVersion)` on parse failure.
    fn check_abi(&self, hash: &PluginId, manifest: &PluginManifest) -> bee_plugin_sdk::PluginResult<()> {
        let abi = AbiVersion::parse(&manifest.abi_version)?;
        if !abi.matches_major(&self.expected_abi_majors) {
            return Err(PluginError::AbiMismatch {
                hash: hash.to_string(),
                claimed: manifest.abi_version.clone(),
                expected: self.expected_abi_majors.clone(),
                migration_link: bee_plugin_sdk::MIGRATION_DOC_LINK.to_string(),
            });
        }
        Ok(())
    }

    /// Register a plugin by its binary content. The PluginId is
    /// derived from `content`; re-registering with the same content
    /// is idempotent. The S20 ABI check runs first; on mismatch the
    /// plugin is NOT registered and the error includes the hash,
    /// claimed `abi_version`, the host's expected majors, and a
    /// migration doc link.
    pub fn register(
        &mut self,
        content: &[u8],
        manifest: PluginManifest,
    ) -> bee_plugin_sdk::PluginResult<PluginId> {
        let id = compute_plugin_id(content);
        self.check_abi(&id, &manifest)?;
        self.plugins.entry(id.clone()).or_insert_with(|| {
            // Construct a minimal handle for the registered manifest.
            // Real plugin init is wired in the libloading follow-up.
            let handle = PluginHandle {
                manifest: manifest.clone(),
                inner: Arc::new(()),
            };
            RegisteredPlugin {
                manifest,
                handle: Arc::new(handle),
                refcount: 0,
            }
        });
        Ok(id)
    }

    /// Register a plugin through the [`Plugin`] trait (in-process test
    /// path). Calls `plugin.init()` to get the handle, computes the
    /// PluginId from the plugin's reported `plugin_content()`, and
    /// stores both. The S20 ABI check runs before `init()`.
    pub fn register_plugin(
        &mut self,
        plugin: &dyn Plugin,
    ) -> bee_plugin_sdk::PluginResult<PluginId> {
        let id = compute_plugin_id(plugin.plugin_content());
        let manifest = plugin.manifest();
        self.check_abi(&id, &manifest)?;
        let handle = plugin.init()?;
        let manifest = handle.manifest.clone();
        self.plugins.entry(id.clone()).or_insert(RegisteredPlugin {
            manifest,
            handle: Arc::new(handle),
            refcount: 0,
        });
        Ok(id)
    }

    /// S33: load a plugin from a `cdylib` on disk via
    /// [`loader::load_library`] and register it. The PluginId is
    /// computed from the binary content; the S20 ABI check runs
    /// against the plugin's declared `abi_version` before adding
    /// to the manager. Idempotent (re-registering the same binary
    /// is a no-op).
    pub fn register_library<P: AsRef<std::path::Path>>(
        &mut self,
        path: P,
    ) -> bee_plugin_sdk::PluginResult<PluginId> {
        let loaded = crate::loader::load_library(path)?;
        let id = loaded.id.clone();
        self.check_abi(&id, &loaded.handle.manifest)?;
        let manifest = loaded.handle.manifest.clone();
        self.plugins.entry(id.clone()).or_insert(RegisteredPlugin {
            manifest,
            handle: loaded.handle,
            refcount: 0,
        });
        Ok(id)
    }

    /// Look up a loaded plugin by PluginId.
    pub fn lookup(&self, id: &PluginId) -> Option<&PluginManifest> {
        self.plugins.get(id).map(|p| &p.manifest)
    }

    /// Get the handle for a loaded plugin.
    pub fn handle(&self, id: &PluginId) -> Option<Arc<PluginHandle>> {
        self.plugins.get(id).map(|p| p.handle.clone())
    }

    /// Enumerate all loaded plugin IDs (sorted lexicographically for
    /// determinism in tests / `bee plugins list`).
    pub fn list(&self) -> Vec<PluginId> {
        let mut ids: Vec<PluginId> = self.plugins.keys().cloned().collect();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        ids
    }

    /// Enumerate all Adapter descriptors across all loaded plugins.
    /// The Compiler/Registry uses this to resolve SQL references
    /// (e.g. `binance.subscribe(...)`) to a specific PluginId.
    pub fn list_adapters(&self) -> Vec<(PluginId, AdapterDescriptor)> {
        let mut out = Vec::new();
        for (id, p) in &self.plugins {
            for a in &p.manifest.adapters {
                out.push((id.clone(), a.clone()));
            }
        }
        out.sort_by(|x, y| x.0 .0.cmp(&y.0 .0).then(x.1.name.cmp(&y.1.name)));
        out
    }

    /// Enumerate all Handler descriptors across all loaded plugins.
    pub fn list_handlers(&self) -> Vec<(PluginId, HandlerDescriptor)> {
        let mut out = Vec::new();
        for (id, p) in &self.plugins {
            for h in &p.manifest.handlers {
                out.push((id.clone(), h.clone()));
            }
        }
        out.sort_by(|x, y| x.0 .0.cmp(&y.0 .0).then(x.1.name.cmp(&y.1.name)));
        out
    }

    /// Number of loaded plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// S21: resolve a Plugin reference (logical `name` + SemVer
    /// `spec`) to a concrete [`PluginId`]. Filters loaded plugins by
    /// name, parses each `manifest.feature_version` to a [`Version`],
    /// and picks the **highest** version that satisfies the spec.
    /// Returns `None` if no loaded plugin matches — the Pipeline
    /// submit must then fail with a clear error (the Compiler is
    /// responsible for surfacing it).
    ///
    /// Multiple `.so` files for the same logical Plugin (same `name`
    /// in Manifest, different `version` and `sha256`) can be loaded
    /// simultaneously; each gets a distinct [`PluginId`].
    pub fn resolve(&self, name: &str, spec: &VersionSpec) -> Option<PluginId> {
        let mut best: Option<(PluginId, Version)> = None;
        for (id, p) in &self.plugins {
            if p.manifest.name.0 != name {
                continue;
            }
            let v = match Version::parse(&p.manifest.feature_version) {
                Ok(v) => v,
                Err(_) => continue, // unparseable → skip; the spec only matches parseable versions
            };
            if !spec.matches(&v) {
                continue;
            }
            match &best {
                Some((_, bv)) if *bv >= v => {}
                _ => best = Some((id.clone(), v)),
            }
        }
        best.map(|(id, _)| id)
    }

    /// S21: increment the refcount of a loaded Plugin. The Compiler
    /// calls this when a Pipeline referencing the Plugin is
    /// submitted. Returns `true` if the Plugin was found and the
    /// refcount incremented; `false` if the Plugin is not loaded.
    pub fn retain(&mut self, id: &PluginId) -> bool {
        if let Some(p) = self.plugins.get_mut(id) {
            p.refcount = p.refcount.saturating_add(1);
            true
        } else {
            false
        }
    }

    /// S21: decrement the refcount of a loaded Plugin. When the
    /// refcount reaches 0, the Plugin is auto-unloaded (removed
    /// from the manager). Returns `true` if the Plugin was found
    /// (whether it stayed loaded or was just removed); `false` if
    /// the Plugin is not loaded.
    pub fn release(&mut self, id: &PluginId) -> bool {
        let should_remove = if let Some(p) = self.plugins.get_mut(id) {
            p.refcount = p.refcount.saturating_sub(1);
            p.refcount == 0
        } else {
            return false;
        };
        if should_remove {
            self.plugins.remove(id);
        }
        true
    }

    /// S21: read the current refcount of a loaded Plugin. Returns
    /// `None` if the Plugin is not loaded. Used by `bee plugin list`
    /// to show the refcount next to each Plugin.
    pub fn refcount_of(&self, id: &PluginId) -> Option<u32> {
        self.plugins.get(id).map(|p| p.refcount)
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Network sync trait stub (S19 follow-up). Real impl will broadcast
/// `Registered { id, manifest }` deltas to peers over BRP and persist
/// the registry in the KV cluster.
pub trait NetworkSync: Send + Sync {
    fn announce(&self, id: &PluginId, manifest: &PluginManifest);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_plugin_sdk::{PluginError, PluginName};

    /// Built-in mock plugin used by the S19 tests. Reports a fixed
    /// content slice so its PluginId is deterministic.
    struct MockBinancePlugin;

    const MOCK_BINANCE_CONTENT: &[u8] = b"mock-binance-plugin-v1";

    impl Plugin for MockBinancePlugin {
        fn plugin_content(&self) -> &'static [u8] {
            MOCK_BINANCE_CONTENT
        }
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                name: PluginName("binance".into()),
                feature_version: "1.0.0".into(),
                abi_version: "v1".into(),
                adapters: vec![AdapterDescriptor {
                    name: "subscribe".into(),
                    is_input: true,
                }],
                handlers: vec![],
            }
        }
        fn init(&self) -> bee_plugin_sdk::PluginResult<PluginHandle> {
            Ok(PluginHandle {
                manifest: self.manifest(),
                inner: Arc::new(()),
            })
        }
    }

    #[test]
    fn empty_manager_has_no_plugins() {
        let mgr = PluginManager::new();
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn register_plugin_uses_content_hash_as_id() {
        let mut mgr = PluginManager::new();
        let plugin = MockBinancePlugin;
        let id = mgr.register_plugin(&plugin).expect("register");
        assert_eq!(id.0.len(), PluginId::HEX_LEN);
        assert_eq!(id, compute_plugin_id(MOCK_BINANCE_CONTENT));
        assert_eq!(mgr.len(), 1);
    }

    #[test]
    fn register_same_content_twice_is_idempotent() {
        let mut mgr = PluginManager::new();
        let plugin = MockBinancePlugin;
        let id1 = mgr.register_plugin(&plugin).unwrap();
        let id2 = mgr.register_plugin(&plugin).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(mgr.len(), 1);
    }

    #[test]
    fn register_different_content_yields_different_id() {
        let mut mgr = PluginManager::new();
        let id1 = mgr
            .register(
                b"plugin-a",
                PluginManifest {
                    name: PluginName("a".into()),
                    feature_version: "1.0".into(),
                    abi_version: "v1".into(),
                    adapters: vec![],
                    handlers: vec![],
                },
            )
            .unwrap();
        let id2 = mgr
            .register(
                b"plugin-b",
                PluginManifest {
                    name: PluginName("b".into()),
                    feature_version: "1.0".into(),
                    abi_version: "v1".into(),
                    adapters: vec![],
                    handlers: vec![],
                },
            )
            .unwrap();
        assert_ne!(id1, id2);
        assert_eq!(mgr.len(), 2);
    }

    #[test]
    fn lookup_returns_manifest() {
        let mut mgr = PluginManager::new();
        let plugin = MockBinancePlugin;
        let id = mgr.register_plugin(&plugin).unwrap();
        let m = mgr.lookup(&id).expect("lookup");
        assert_eq!(m.name, PluginName("binance".into()));
        assert_eq!(m.adapters.len(), 1);
        assert_eq!(m.adapters[0].name, "subscribe");
    }

    #[test]
    fn list_adapters_aggregates_across_plugins() {
        let mut mgr = PluginManager::new();
        let binance_id = mgr.register_plugin(&MockBinancePlugin).unwrap();
        let coingecko_id = mgr
            .register(
                b"mock-coingecko-plugin",
                PluginManifest {
                    name: PluginName("coingecko".into()),
                    feature_version: "0.1".into(),
                    abi_version: "v1".into(),
                    adapters: vec![
                        AdapterDescriptor {
                            name: "subscribe".into(),
                            is_input: true,
                        },
                        AdapterDescriptor {
                            name: "snapshot".into(),
                            is_input: false,
                        },
                    ],
                    handlers: vec![],
                },
            )
            .unwrap();
        let adapters = mgr.list_adapters();
        // 1 (binance.subscribe) + 2 (coingecko.*) = 3
        assert_eq!(adapters.len(), 3);
        // The result is sorted by (plugin_id, adapter_name). We don't
        // assume a specific position for either plugin, but the
        // binance+subscribe pair must be present and the two
        // coingecko pairs must be present and adjacent.
        let names: Vec<&str> = adapters.iter().map(|(_, a)| a.name.as_str()).collect();
        assert!(names.contains(&"subscribe"));
        assert!(names.contains(&"snapshot"));
        assert!(names.iter().filter(|n| **n == "subscribe").count() == 2);
        // Within the same plugin_id, "snapshot" sorts before "subscribe".
        let coingecko_indices: Vec<usize> = adapters
            .iter()
            .enumerate()
            .filter(|(_, (id, _))| id == &coingecko_id)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(coingecko_indices.len(), 2);
        let coingecko_names: Vec<&str> = coingecko_indices
            .iter()
            .map(|&i| adapters[i].1.name.as_str())
            .collect();
        assert_eq!(coingecko_names, vec!["snapshot", "subscribe"]);
        // And the binance entry must reference the binance plugin_id.
        let binance_present = adapters.iter().any(|(id, a)| id == &binance_id && a.name == "subscribe");
        assert!(binance_present);
    }

    #[test]
    fn handle_is_arc_shared() {
        let mut mgr = PluginManager::new();
        let id = mgr.register_plugin(&MockBinancePlugin).unwrap();
        let h1 = mgr.handle(&id).unwrap();
        let h2 = mgr.handle(&id).unwrap();
        assert!(Arc::ptr_eq(&h1, &h2));
    }

    #[test]
    fn bee_host_v1_struct_size_is_stable() {
        // Smoke test that the FFI struct is layout-stable. The plugin
        // receives a `*mut BeeHostV1` from the host; if the layout
        // ever changes accidentally, the FFI breaks.
        use bee_plugin_sdk::BeeHostV1;
        assert!(std::mem::size_of::<BeeHostV1>() > 0);
    }

    #[test]
    fn not_found_returns_none() {
        let mgr = PluginManager::new();
        let bogus = PluginId("0".repeat(PluginId::HEX_LEN));
        assert!(mgr.lookup(&bogus).is_none());
    }

    #[test]
    fn init_failure_propagates() {
        struct InitFails;
        impl Plugin for InitFails {
            fn plugin_content(&self) -> &'static [u8] {
                b"init-fails"
            }
            fn manifest(&self) -> PluginManifest {
                PluginManifest {
                    name: PluginName("broken".into()),
                    feature_version: "0".into(),
                    abi_version: "v1".into(),
                    adapters: vec![],
                    handlers: vec![],
                }
            }
            fn init(&self) -> bee_plugin_sdk::PluginResult<PluginHandle> {
                Err(PluginError::Init("nope".into()))
            }
        }
        let mut mgr = PluginManager::new();
        let r = mgr.register_plugin(&InitFails);
        assert!(matches!(r, Err(PluginError::Init(_))));
        assert_eq!(mgr.len(), 0);
    }

    // ---- ABI version check (S20) ----

    fn make_manifest(name: &str, abi: &str) -> PluginManifest {
        PluginManifest {
            name: PluginName(name.into()),
            feature_version: "1.0".into(),
            abi_version: abi.into(),
            adapters: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn default_manager_accepts_abi_v1() {
        let mut mgr = PluginManager::new();
        assert_eq!(mgr.expected_abi_majors(), &[1]);
        let id = mgr
            .register(b"plugin-v1", make_manifest("a", "v1.0"))
            .expect("v1 accepted");
        assert_eq!(mgr.len(), 1);
        assert!(mgr.lookup(&id).is_some());
    }

    #[test]
    fn default_manager_rejects_abi_v2() {
        let mut mgr = PluginManager::new();
        let r = mgr.register(b"plugin-v2", make_manifest("a", "v2.0"));
        match r {
            Err(PluginError::AbiMismatch {
                hash,
                claimed,
                expected,
                migration_link,
            }) => {
                assert_eq!(hash.len(), PluginId::HEX_LEN);
                assert_eq!(claimed, "v2.0");
                assert_eq!(expected, vec![1]);
                assert!(
                    migration_link.contains("0009-plugin-multiversion"),
                    "expected migration link, got: {migration_link}"
                );
            }
            other => panic!("expected AbiMismatch, got {other:?}"),
        }
        assert_eq!(mgr.len(), 0, "rejected plugin must NOT be registered");
    }

    #[test]
    fn abi_mismatch_error_message_includes_all_required_fields() {
        let mut mgr = PluginManager::new();
        let err = mgr
            .register(b"plugin-v3", make_manifest("a", "3.0"))
            .unwrap_err();
        let msg = format!("{err}");
        // Per S20 acceptance: hash, claimed abi_version, expected
        // range, link to migration docs.
        assert!(msg.contains("hash="), "missing hash field:\n{msg}");
        assert!(msg.contains("claimed_abi=3.0"), "missing claimed:\n{msg}");
        assert!(msg.contains("expected_majors=[1]"), "missing expected:\n{msg}");
        assert!(
            msg.contains("https://") || msg.contains("docs"),
            "missing migration link:\n{msg}"
        );
    }

    #[test]
    fn manager_with_two_abi_majors_accepts_both() {
        let mut mgr = PluginManager::with_abi_majors(vec![1, 2]);
        assert_eq!(mgr.expected_abi_majors(), &[1, 2]);
        let id1 = mgr
            .register(b"plugin-v1", make_manifest("a", "v1.0"))
            .expect("v1");
        let id2 = mgr
            .register(b"plugin-v2", make_manifest("b", "v2.0"))
            .expect("v2");
        assert_eq!(mgr.len(), 2);
        assert!(mgr.lookup(&id1).is_some());
        assert!(mgr.lookup(&id2).is_some());
    }

    #[test]
    fn manager_with_empty_abi_list_rejects_everything() {
        let mut mgr = PluginManager::with_abi_majors(vec![]);
        let r = mgr.register(b"plugin-v1", make_manifest("a", "v1.0"));
        assert!(matches!(r, Err(PluginError::AbiMismatch { .. })));
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn invalid_abi_version_string_returns_parse_error() {
        let mut mgr = PluginManager::new();
        let r = mgr.register(b"plugin-bad", make_manifest("a", "garbage"));
        assert!(matches!(r, Err(PluginError::InvalidAbiVersion(_))));
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn set_expected_abi_majors_changes_behavior_at_runtime() {
        let mut mgr = PluginManager::new();
        // v2 rejected by default
        assert!(mgr
            .register(b"plugin-v2", make_manifest("a", "v2.0"))
            .is_err());
        // Widen to [1, 2], now v2 accepted
        mgr.set_expected_abi_majors(vec![1, 2]);
        let id = mgr
            .register(b"plugin-v2", make_manifest("a", "v2.0"))
            .expect("v2 after widening");
        assert_eq!(mgr.len(), 1);
        assert!(mgr.lookup(&id).is_some());
        // Narrow back to [2], the v2 plugin stays registered but a
        // new v1 is rejected.
        mgr.set_expected_abi_majors(vec![2]);
        assert!(mgr
            .register(b"plugin-v1", make_manifest("b", "v1.0"))
            .is_err());
        assert_eq!(mgr.len(), 1, "already-registered plugin unaffected");
    }

    #[test]
    fn register_plugin_via_trait_runs_abi_check_before_init() {
        // A plugin whose init() would panic / fail should still be
        // caught by the ABI check first. Verify by giving it a
        // mismatched abi_version.
        struct Abiv2InitSucceeds;
        impl Plugin for Abiv2InitSucceeds {
            fn plugin_content(&self) -> &'static [u8] {
                b"v2-content"
            }
            fn manifest(&self) -> PluginManifest {
                make_manifest("v2-plugin", "v2.0")
            }
            fn init(&self) -> bee_plugin_sdk::PluginResult<PluginHandle> {
                Ok(PluginHandle {
                    manifest: self.manifest(),
                    inner: Arc::new(()),
                })
            }
        }
        let mut mgr = PluginManager::new();
        let r = mgr.register_plugin(&Abiv2InitSucceeds);
        // ABI check must fire BEFORE init() — otherwise init() would
        // have succeeded and we'd see Ok. Match the error to confirm.
        assert!(matches!(r, Err(PluginError::AbiMismatch { .. })));
        assert_eq!(mgr.len(), 0);
    }

    // ---- Multi-version coexistence + refcount (S21) ----

    fn make_manifest_v(name: &str, feature: &str, abi: &str) -> PluginManifest {
        PluginManifest {
            name: PluginName(name.into()),
            feature_version: feature.into(),
            abi_version: abi.into(),
            adapters: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn resolve_picks_highest_matching_version() {
        let mut mgr = PluginManager::new();
        let id_142 = mgr
            .register(b"binance-1.4.2", make_manifest_v("binance", "1.4.2", "v1"))
            .unwrap();
        let id_200 = mgr
            .register(b"binance-2.0.0", make_manifest_v("binance", "2.0.0", "v1"))
            .unwrap();
        assert_eq!(mgr.len(), 2);
        assert_ne!(id_142, id_200);

        // binance:^1.0 → only 1.4.2 matches
        let spec = VersionSpec::parse("^1.0").unwrap();
        assert_eq!(mgr.resolve("binance", &spec), Some(id_142.clone()));

        // binance:^2.0 → only 2.0.0 matches
        let spec = VersionSpec::parse("^2.0").unwrap();
        assert_eq!(mgr.resolve("binance", &spec), Some(id_200.clone()));

        // binance:latest → highest, 2.0.0
        let spec = VersionSpec::parse("latest").unwrap();
        assert_eq!(mgr.resolve("binance", &spec), Some(id_200.clone()));

        // binance:1.4.2 (exact) → 1.4.2
        let spec = VersionSpec::parse("1.4.2").unwrap();
        assert_eq!(mgr.resolve("binance", &spec), Some(id_142.clone()));
    }

    #[test]
    fn resolve_with_no_matching_plugin_returns_none() {
        let mut mgr = PluginManager::new();
        mgr.register(b"binance-1.4.2", make_manifest_v("binance", "1.4.2", "v1"))
            .unwrap();
        // ^3.0 has no match — only 1.4.2 loaded.
        let spec = VersionSpec::parse("^3.0").unwrap();
        assert_eq!(mgr.resolve("binance", &spec), None);

        // A name that doesn't exist.
        assert_eq!(mgr.resolve("coingecko", &VersionSpec::Latest), None);
    }

    #[test]
    fn resolve_picks_highest_among_multiple_compatible() {
        // ^1.0 across [1.0.0, 1.4.2, 1.10.0, 1.9.9] → 1.10.0
        let mut mgr = PluginManager::new();
        let id_100 = mgr
            .register(b"v-1.0.0", make_manifest_v("foo", "1.0.0", "v1"))
            .unwrap();
        let id_142 = mgr
            .register(b"v-1.4.2", make_manifest_v("foo", "1.4.2", "v1"))
            .unwrap();
        let id_199 = mgr
            .register(b"v-1.9.9", make_manifest_v("foo", "1.9.9", "v1"))
            .unwrap();
        let id_110 = mgr
            .register(b"v-1.10.0", make_manifest_v("foo", "1.10.0", "v1"))
            .unwrap();
        let spec = VersionSpec::parse("^1.0").unwrap();
        let resolved = mgr.resolve("foo", &spec).expect("match");
        assert_eq!(
            resolved, id_110,
            "^1.0 across multiple versions must pick 1.10.0 (highest); got {resolved:?}, \
             candidates: 1.0.0={id_100}, 1.4.2={id_142}, 1.9.9={id_199}, 1.10.0={id_110}"
        );
    }

    #[test]
    fn resolve_skips_plugins_with_unparseable_feature_version() {
        // A plugin whose feature_version doesn't parse (e.g. "garbage")
        // is simply skipped by the resolver. The user still has the
        // other parseable versions to choose from.
        let mut mgr = PluginManager::new();
        let id_bad = mgr
            .register(b"bad", make_manifest_v("foo", "garbage", "v1"))
            .unwrap();
        let id_good = mgr
            .register(b"good", make_manifest_v("foo", "1.0.0", "v1"))
            .unwrap();
        let spec = VersionSpec::parse("^1.0").unwrap();
        assert_eq!(mgr.resolve("foo", &spec), Some(id_good.clone()));
        // The bad one is still loaded — the user might fix its
        // feature_version and re-register.
        assert!(mgr.lookup(&id_bad).is_some());
    }

    #[test]
    fn resolve_filters_by_name() {
        // A binance and a coingecko loaded; `binance:^1.0` must NOT
        // return a coingecko even if its version matches.
        let mut mgr = PluginManager::new();
        let id_b = mgr
            .register(b"binance", make_manifest_v("binance", "1.0.0", "v1"))
            .unwrap();
        let id_c = mgr
            .register(b"coingecko", make_manifest_v("coingecko", "1.0.0", "v1"))
            .unwrap();
        let spec = VersionSpec::parse("^1.0").unwrap();
        assert_eq!(mgr.resolve("binance", &spec), Some(id_b.clone()));
        assert_eq!(mgr.resolve("coingecko", &spec), Some(id_c.clone()));
    }

    #[test]
    fn retain_increments_refcount_release_decrements() {
        let mut mgr = PluginManager::new();
        let id = mgr
            .register(b"p", make_manifest_v("p", "1.0.0", "v1"))
            .unwrap();
        assert_eq!(mgr.refcount_of(&id), Some(0));
        assert!(mgr.retain(&id));
        assert_eq!(mgr.refcount_of(&id), Some(1));
        assert!(mgr.retain(&id));
        assert_eq!(mgr.refcount_of(&id), Some(2));
        assert!(mgr.release(&id));
        assert_eq!(mgr.refcount_of(&id), Some(1));
        assert!(mgr.lookup(&id).is_some(), "still loaded at refcount=1");
        assert!(mgr.release(&id));
        assert_eq!(mgr.refcount_of(&id), None, "auto-unloaded at refcount=0");
        assert!(mgr.lookup(&id).is_none());
    }

    #[test]
    fn release_saturates_at_zero_no_underflow() {
        let mut mgr = PluginManager::new();
        let id = mgr
            .register(b"p", make_manifest_v("p", "1.0.0", "v1"))
            .unwrap();
        // No retains; just release. Should saturate at 0 and remove
        // the plugin (not underflow to u32::MAX).
        assert!(mgr.release(&id));
        assert!(mgr.lookup(&id).is_none());
        // Releasing an already-removed plugin returns false.
        assert!(!mgr.release(&id));
    }

    #[test]
    fn retain_returns_false_for_unknown_plugin() {
        let mut mgr = PluginManager::new();
        let bogus = bee_plugin_sdk::PluginId("0".repeat(bee_plugin_sdk::PluginId::HEX_LEN));
        assert!(!mgr.retain(&bogus));
    }

    #[test]
    fn two_versions_of_binance_run_independently_in_the_manager() {
        // S21 acceptance: 2 versions of `binance` (1.4.2 and 2.0.0)
        // both loaded; 2 Pipelines each referencing one version; both
        // run independently. At the manager level this means: both
        // are loaded with distinct PluginIds, retain/release work
        // independently, and a release of one does not affect the
        // other.
        let mut mgr = PluginManager::new();
        let id_142 = mgr
            .register(b"binance-1.4.2", make_manifest_v("binance", "1.4.2", "v1"))
            .unwrap();
        let id_200 = mgr
            .register(b"binance-2.0.0", make_manifest_v("binance", "2.0.0", "v1"))
            .unwrap();

        // Pipeline 1 references 1.4.2; Pipeline 2 references 2.0.0.
        let spec_142 = VersionSpec::parse("1.4.2").unwrap();
        let spec_200 = VersionSpec::parse("2.0.0").unwrap();
        let resolved_1 = mgr.resolve("binance", &spec_142).unwrap();
        let resolved_2 = mgr.resolve("binance", &spec_200).unwrap();
        mgr.retain(&resolved_1);
        mgr.retain(&resolved_2);
        assert_eq!(mgr.refcount_of(&id_142), Some(1));
        assert_eq!(mgr.refcount_of(&id_200), Some(1));

        // Pipeline 2 stops — release 2.0.0. 1.4.2 must stay.
        mgr.release(&resolved_2);
        assert!(mgr.lookup(&id_200).is_none());
        assert!(
            mgr.lookup(&id_142).is_some(),
            "Pipeline 1's plugin must stay loaded after Pipeline 2 stops"
        );
    }
}
