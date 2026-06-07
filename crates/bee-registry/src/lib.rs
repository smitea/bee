//! `bee-registry` — Bee 注册中心。
//!
//! 管理 Plugin 加载 / ABI 校验 / 哈希校验 / 网络同步 / 路由表广播。
//! Registry 是 trait,具体实现可插拔 (本地、etcd 风格、内存测试桩)。
//!
//! S19 起实现 [`PluginManager`] 与 [`NetworkSync`]。

use std::collections::HashMap;
use std::sync::Arc;

use bee_plugin_sdk::{
    compute_plugin_id, AbiVersion, AdapterDescriptor, HandlerDescriptor, Plugin, PluginError,
    PluginHandle, PluginId, PluginManifest,
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
}
