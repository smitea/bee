//! S-2 GUI data-management facade.
//!
//! Wraps `DatasourceRegistry` in `Arc<Mutex<_>>` and exposes a small,
//! GUI-friendly surface. For MVP the GUI holds its own local registry
//! (matching the `bee datasource …` CLI pattern: in MVP the registry
//! is process-local; production wires it to AdminServer RPC + Raft KV).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use bee_control::datasource::{Datasource, DatasourceRegistry};
use bee_plugin_sdk::{PluginId, VersionSpec};

#[derive(Clone)]
pub struct DataMgmtState {
    inner: Arc<Mutex<DatasourceRegistry>>,
}

impl Default for DataMgmtState {
    fn default() -> Self {
        Self::new()
    }
}

impl DataMgmtState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(DatasourceRegistry::new())),
        }
    }

    pub fn list(&self) -> Vec<Datasource> {
        self.inner
            .lock()
            .expect("DataMgmtState poisoned")
            .list(None)
            .into_iter()
            .cloned()
            .collect()
    }

    pub fn create(
        &self,
        name: String,
        adapter: String,
        version_spec: String,
        config: String,
        tenant: u16,
    ) -> Result<Datasource, String> {
        let plugin_id = derived_plugin_id(&adapter, &version_spec);
        let spec = VersionSpec::parse(&version_spec).map_err(|e| e.to_string())?;
        let ds = Datasource::new(name, tenant, adapter, plugin_id, spec, config);
        let mut g = self.inner.lock().expect("DataMgmtState poisoned");
        g.create(ds)
            .map(|d| d.clone())
            .map_err(|e| format!("{e:?}"))
    }

    pub fn pause(&self, tenant: u16, name: &str) -> Result<(), String> {
        let mut g = self.inner.lock().expect("DataMgmtState poisoned");
        g.pause(tenant, name)
            .map(|_| ())
            .map_err(|e| format!("{:?}", e))
    }

    pub fn resume(&self, tenant: u16, name: &str) -> Result<(), String> {
        let mut g = self.inner.lock().expect("DataMgmtState poisoned");
        g.resume(tenant, name)
            .map(|_| ())
            .map_err(|e| format!("{:?}", e))
    }

    pub fn delete(&self, tenant: u16, name: &str) -> Result<(), String> {
        let mut g = self.inner.lock().expect("DataMgmtState poisoned");
        g.delete(tenant, name)
            .map(|_| ())
            .map_err(|e| format!("{:?}", e))
    }

    pub fn inspect(&self, tenant: u16, name: &str) -> Option<DatasourceInspectionView> {
        let mut g = self.inner.lock().expect("DataMgmtState poisoned");
        let ds = g.get(tenant, name)?.clone();
        let referencing_job_count = g.referencing_job_count(tenant, name);
        let health = g.health(tenant, name).ok().map(|h| DatasourceHealthView {
            connection_success_total: h.connection_success_total.load(Ordering::Relaxed),
            connection_failure_total: h.connection_failure_total.load(Ordering::Relaxed),
            last_success_at_ms: h.last_success_at_ms.load(Ordering::Relaxed),
            last_failure_at_ms: h.last_failure_at_ms.load(Ordering::Relaxed),
            referencing_job_count,
        });
        Some(DatasourceInspectionView {
            datasource: ds,
            health,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DatasourceInspectionView {
    pub datasource: Datasource,
    pub health: Option<DatasourceHealthView>,
}

#[derive(Debug, Clone)]
pub struct DatasourceHealthView {
    pub connection_success_total: u64,
    pub connection_failure_total: u64,
    pub last_success_at_ms: u64,
    pub last_failure_at_ms: u64,
    pub referencing_job_count: usize,
}

/// MVP-only deterministic plugin_id. Production resolves via
/// PluginManager + content hash. This avoids pulling the full
/// PluginManager into the GUI just for the form preview.
fn derived_plugin_id(adapter: &str, version_spec: &str) -> PluginId {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    adapter.hash(&mut h);
    version_spec.hash(&mut h);
    let raw = h.finish();
    PluginId(format!("mvp-{:016x}", raw))
}

// Re-export so the GUI doesn't have to know bee_control::datasource::*
pub use bee_control::datasource::DatasourceStatus as GuiDatasourceStatus;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_list() {
        let s = DataMgmtState::new();
        s.create(
            "binance".into(),
            "binance_subscribe".into(),
            "^1.0".into(),
            r#"{"base_url":"wss://api.binance.com"}"#.into(),
            0,
        )
        .expect("create");
        let list = s.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "binance");
    }

    #[test]
    fn create_pause_resume_delete() {
        let s = DataMgmtState::new();
        s.create("a".into(), "ad".into(), "1.0".into(), "{}".into(), 0)
            .unwrap();
        assert!(s.pause(0, "a").is_ok());
        assert!(s.resume(0, "a").is_ok());
        assert!(s.delete(0, "a").is_ok());
        assert!(s.list().is_empty());
    }

    #[test]
    fn inspect_returns_health_view() {
        let s = DataMgmtState::new();
        s.create("a".into(), "ad".into(), "1.0".into(), "{}".into(), 0)
            .unwrap();
        let view = s.inspect(0, "a");
        assert!(view.is_some());
        assert_eq!(view.unwrap().datasource.name, "a");
    }

    #[test]
    fn derived_plugin_id_is_deterministic() {
        let a = derived_plugin_id("binance_subscribe", "^1.0");
        let b = derived_plugin_id("binance_subscribe", "^1.0");
        assert_eq!(a, b);
        let c = derived_plugin_id("binance_subscribe", "^2.0");
        assert_ne!(a, c);
    }
}