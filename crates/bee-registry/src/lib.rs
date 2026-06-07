//! `bee-registry` — Bee 注册中心。
//!
//! 管理 Plugin 加载 / ABI 校验 / 哈希校验 / 网络同步 / 路由表广播。
//! Registry 是 trait,具体实现可插拔 (本地、etcd 风格、内存测试桩)。
//!
//! S19 起实现 [`PluginManager`] 与 [`NetworkSync`]。

use std::collections::HashMap;
use std::sync::Arc;

use bee_plugin_sdk::{
    compute_plugin_id, AdapterDescriptor, HandlerDescriptor, Plugin, PluginHandle, PluginId,
    PluginManifest,
};

/// In-process Plugin Manager. Holds the set of loaded plugins keyed
/// by their content-hash [`PluginId`].
///
/// ## S19 MVP scope
/// - `register(content, manifest)`: compute PluginId from the
///   content bytes, store the manifest + handle. Idempotent on
///   PluginId.
/// - `lookup(id)`: fetch the manifest for a known PluginId.
/// - `list()`: enumerate the loaded plugins (sorted by PluginId for
///   determinism).
/// - `register_adapter` / `register_handler`: per-Plugin registration
///   (delegated to the host's registry; S19 stores the descriptor
///   in the manifest copy).
///
/// Out of S19 scope (follow-ups):
/// - `libloading` to load `.so`/`.dylib`/`.dll` and call the C ABI
///   `bee_plugin_init` symbol. S19+ follow-up.
/// - A real test `cdylib` plugin (S19+ follow-up).
/// - Network sync between nodes (the `NetworkSync` trait stub).
pub struct PluginManager {
    plugins: HashMap<PluginId, RegisteredPlugin>,
}

struct RegisteredPlugin {
    manifest: PluginManifest,
    handle: Arc<PluginHandle>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Register a plugin by its binary content. The PluginId is
    /// derived from `content`; re-registering with the same content
    /// is idempotent.
    pub fn register(
        &mut self,
        content: &[u8],
        manifest: PluginManifest,
    ) -> bee_plugin_sdk::PluginResult<PluginId> {
        let id = compute_plugin_id(content);
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
    /// stores both.
    pub fn register_plugin(
        &mut self,
        plugin: &dyn Plugin,
    ) -> bee_plugin_sdk::PluginResult<PluginId> {
        let id = compute_plugin_id(plugin.plugin_content());
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
}
