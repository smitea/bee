//! Datasource managed entity (S29, ADR-0010).
//!
//! A Datasource is the **management view** on top of the runtime
//! Phase-with-Adapter concept (ADR-0002). The runtime still sees
//! a Phase; the user-facing API is `Datasource { name, tenant,
//! adapter, plugin_id, version_spec, config, status, ... }`
//! registered by an admin and referenced by Pipeline Authors via
//! the `use <name>;` SQL directive.
//!
//! ## S29 MVP scope
//! - [`Datasource`] data model + [`DatasourceStatus`] enum
//! - [`DatasourceRegistry`] with create/get/list/pause/resume/delete
//! - Stored in-memory in the MVP. The KV persistence path
//!   (`ds/{tenant}/{name}` keys, per spec) is the S30+ follow-up.
//! - `tenant: u16` on the Datasource (MVP: struct field only,
//!   no ACL enforcement).
//!
//! Out of S29 scope:
//! - `bee datasource test <name>` (probe via Plugin's
//!   `test_connection` method) — deferred (needs real
//!   libloading + Plugin trait extension).
//! - `bee datasource pause <name>` triggers Draining on all
//!   referencing Jobs — deferred (S31 per the roadmap).

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use bee_plugin_sdk::{PluginId, VersionSpec};

/// Lifecycle state of a Datasource. `Active` = Producer running
/// and Subscribers can attach. `Paused` = Producer stopped;
/// existing Subscribers continue to receive the cached stream
/// (per ADR-0010 pause semantics). New Pipelines can't `use` it.
/// `Disabled` = tombstoned; same as Paused but the user marked
/// the Datasource for deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DatasourceStatus {
    #[default]
    Active,
    Paused,
    Disabled,
}

impl std::fmt::Display for DatasourceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatasourceStatus::Active => f.write_str("active"),
            DatasourceStatus::Paused => f.write_str("paused"),
            DatasourceStatus::Disabled => f.write_str("disabled"),
        }
    }
}

/// S29 managed-entity data model. The runtime Phase-with-Adapter
/// is a derived view (per ADR-0010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Datasource {
    /// User-facing name (e.g., "binance"). Unique within a tenant.
    pub name: String,
    /// Tenant namespace (`u16`; 0 = global per ADR-0010).
    pub tenant: u16,
    /// Adapter name (the Plugin's logical name, e.g., "binance").
    /// Used by the SQL preprocessor to recognize `<name>.method(...)`
    /// calls after `use <name>;`.
    pub adapter: String,
    /// Resolved PluginId (sha256 hash of the loaded Plugin binary).
    /// The S19 PluginManager resolves this at create time; the
    /// S30+ persistence layer stores it in KV.
    pub plugin_id: PluginId,
    /// The SemVer range used to resolve the Plugin at create time.
    /// When the Pipeline uses `use <name>;` (no pin), this spec
    /// selects the Plugin. When the Pipeline uses
    /// `use <name>@<spec>;`, the pipeline's spec wins.
    pub version_spec: VersionSpec,
    /// Adapter configuration (opaque JSON for MVP).
    pub config: String,
    pub status: DatasourceStatus,
    /// Wall-clock millis. MVP uses SystemTime; production stamps
    /// with a Raft-applied timestamp.
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    /// Node that currently runs the Producer (None = not yet
    /// provisioned, or the Datasource is Paused/Disabled).
    pub owner_node: Option<u32>,
}

impl Datasource {
    /// Convenience: construct a new Datasource with timestamps
    /// set to "now". Used by `DatasourceRegistry::create` and
    /// tests.
    pub fn new(
        name: String,
        tenant: u16,
        adapter: String,
        plugin_id: PluginId,
        version_spec: VersionSpec,
        config: String,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            name,
            tenant,
            adapter,
            plugin_id,
            version_spec,
            config,
            status: DatasourceStatus::default(),
            created_at_ms: now,
            updated_at_ms: now,
            owner_node: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DatasourceError {
    #[error("datasource `{tenant}/{name}` already exists")]
    AlreadyExists { tenant: u16, name: String },
    #[error("datasource `{tenant}/{name}` not found")]
    NotFound { tenant: u16, name: String },
    #[error("datasource `{tenant}/{name}` is {status}, not {expected}")]
    InvalidStatus {
        tenant: u16,
        name: String,
        status: DatasourceStatus,
        expected: &'static str,
    },
    #[error("invalid name `{0}` (empty)")]
    InvalidName(String),
}

pub type DatasourceResult<T> = std::result::Result<T, DatasourceError>;

/// In-memory registry of Datasources. The S30+ persistence layer
/// stores entries in the Raft-replicated KV at `ds/{tenant}/{name}`.
///
/// Keyed by `(tenant, name)`. Operations are local mutations;
/// the S30+ wiring is via the existing KV SM + ControlPlane
/// dispatcher.
pub struct DatasourceRegistry {
    by_key: HashMap<(u16, String), Datasource>,
}

impl Default for DatasourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DatasourceRegistry {
    pub fn new() -> Self {
        Self {
            by_key: HashMap::new(),
        }
    }

    /// Create a Datasource. Errors if `(tenant, name)` already
    /// exists. The caller is responsible for resolving the
    /// `PluginId` (via `PluginManager::resolve`) and choosing the
    /// `version_spec` BEFORE calling `create` — the registry
    /// doesn't validate the plugin_id, just stores it.
    pub fn create(&mut self, ds: Datasource) -> DatasourceResult<Datasource> {
        if ds.name.is_empty() {
            return Err(DatasourceError::InvalidName(ds.name));
        }
        let key = (ds.tenant, ds.name.clone());
        if self.by_key.contains_key(&key) {
            return Err(DatasourceError::AlreadyExists {
                tenant: ds.tenant,
                name: ds.name,
            });
        }
        self.by_key.insert(key, ds.clone());
        Ok(ds)
    }

    pub fn get(&self, tenant: u16, name: &str) -> Option<&Datasource> {
        self.by_key.get(&(tenant, name.to_string()))
    }

    /// List Datasources. If `tenant` is `Some`, filter to that
    /// tenant; otherwise list all.
    pub fn list(&self, tenant: Option<u16>) -> Vec<&Datasource> {
        let mut out: Vec<&Datasource> = self
            .by_key
            .values()
            .filter(|d| tenant.map_or(true, |t| d.tenant == t))
            .collect();
        out.sort_by(|a, b| a.tenant.cmp(&b.tenant).then(a.name.cmp(&b.name)));
        out
    }

    /// Pause an Active Datasource. Errors if not found or not
    /// Active.
    pub fn pause(&mut self, tenant: u16, name: &str) -> DatasourceResult<()> {
        self.set_status(tenant, name, DatasourceStatus::Paused, "active")
    }

    /// Resume a Paused Datasource. Errors if not found or not
    /// Paused.
    pub fn resume(&mut self, tenant: u16, name: &str) -> DatasourceResult<()> {
        self.set_status(tenant, name, DatasourceStatus::Active, "paused")
    }

    fn set_status(
        &mut self,
        tenant: u16,
        name: &str,
        new_status: DatasourceStatus,
        expected: &'static str,
    ) -> DatasourceResult<()> {
        let key = (tenant, name.to_string());
        let ds = self
            .by_key
            .get_mut(&key)
            .ok_or(DatasourceError::NotFound {
                tenant,
                name: name.to_string(),
            })?;
        if format!("{}", ds.status) != expected {
            return Err(DatasourceError::InvalidStatus {
                tenant,
                name: name.to_string(),
                status: ds.status,
                expected,
            });
        }
        ds.status = new_status;
        ds.updated_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Ok(())
    }

    pub fn delete(&mut self, tenant: u16, name: &str) -> DatasourceResult<()> {
        let key = (tenant, name.to_string());
        self.by_key
            .remove(&key)
            .ok_or(DatasourceError::NotFound {
                tenant,
                name: name.to_string(),
            })?;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_plugin_sdk::compute_plugin_id;

    fn test_ds(name: &str, tenant: u16) -> Datasource {
        Datasource::new(
            name.into(),
            tenant,
            "binance".into(),
            compute_plugin_id(b"binance-v1"),
            VersionSpec::Latest,
            "{}".into(),
        )
    }

    #[test]
    fn registry_starts_empty() {
        let r = DatasourceRegistry::new();
        assert!(r.is_empty());
        assert!(r.list(None).is_empty());
    }

    #[test]
    fn create_and_get_round_trip() {
        let mut r = DatasourceRegistry::new();
        let ds = test_ds("binance", 0);
        let ds_id = ds.plugin_id.clone();
        r.create(ds).unwrap();
        let got = r.get(0, "binance").unwrap();
        assert_eq!(got.name, "binance");
        assert_eq!(got.tenant, 0);
        assert_eq!(got.plugin_id, ds_id);
    }

    #[test]
    fn create_duplicate_name_in_same_tenant_errors() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("binance", 0)).unwrap();
        let err = r.create(test_ds("binance", 0)).unwrap_err();
        assert!(matches!(err, DatasourceError::AlreadyExists { .. }));
    }

    #[test]
    fn same_name_in_different_tenants_is_ok() {
        // Tenant namespace is the disambiguator (per ADR-0010).
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("binance", 0)).unwrap();
        r.create(test_ds("binance", 1)).unwrap();
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn list_filters_by_tenant() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("a", 0)).unwrap();
        r.create(test_ds("b", 0)).unwrap();
        r.create(test_ds("c", 1)).unwrap();
        assert_eq!(r.list(None).len(), 3);
        assert_eq!(r.list(Some(0)).len(), 2);
        assert_eq!(r.list(Some(1)).len(), 1);
    }

    #[test]
    fn pause_and_resume_lifecycle() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("d", 0)).unwrap();
        r.pause(0, "d").unwrap();
        assert_eq!(r.get(0, "d").unwrap().status, DatasourceStatus::Paused);
        r.resume(0, "d").unwrap();
        assert_eq!(r.get(0, "d").unwrap().status, DatasourceStatus::Active);
    }

    #[test]
    fn pause_when_already_paused_errors() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("d", 0)).unwrap();
        r.pause(0, "d").unwrap();
        let err = r.pause(0, "d").unwrap_err();
        assert!(matches!(err, DatasourceError::InvalidStatus { .. }));
    }

    #[test]
    fn delete_removes_entry() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("d", 0)).unwrap();
        r.delete(0, "d").unwrap();
        assert!(r.get(0, "d").is_none());
    }

    #[test]
    fn delete_missing_entry_errors() {
        let mut r = DatasourceRegistry::new();
        let err = r.delete(0, "nope").unwrap_err();
        assert!(matches!(err, DatasourceError::NotFound { .. }));
    }

    #[test]
    fn empty_name_rejected() {
        let mut r = DatasourceRegistry::new();
        let err = r.create(test_ds("", 0)).unwrap_err();
        assert!(matches!(err, DatasourceError::InvalidName(_)));
    }
}
