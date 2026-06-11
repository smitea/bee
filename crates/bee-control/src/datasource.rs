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

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use bee_plugin_sdk::{PluginId, VersionSpec};

/// Lifecycle state of a Datasource. `Active` = Producer running
/// and Subscribers can attach. `Paused` = Producer stopped;
/// existing Subscribers continue to receive the cached stream
/// (per ADR-0010 pause semantics). New Pipelines can't `use` it.
/// `Disabled` = tombstoned; same as Paused but the user marked
/// the Datasource for deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Adapter configuration (opaque JSON for MVP). Validated by
    /// [`bee_dsl_sql::preprocess::validate_datasource_config`] on
    /// create: keys like `symbol`, `interval`, `query` are
    /// per-call args and belong at the call site, not in the
    /// Datasource config.
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
    /// S31: per-Datasource health probe state. Keyed by
    /// `(tenant, name)`. The probe runs in the background; this
    /// map is the read/write surface.
    health: HashMap<(u16, String), Arc<DatasourceHealth>>,
    /// S31: reverse index — which Jobs reference a given
    /// Datasource. Updated by the Deployer (not the registry
    /// itself). Used by `inspect` and by the auto-pause Draining
    /// path to enumerate referencing Jobs.
    referencing_jobs: HashMap<(u16, String), HashSet<u32>>,
    /// S29 redo: append-only log of Draining events triggered by
    /// `pause`. The control plane consumes this to migrate
    /// referencing Tasks. In production the events are Raft-
    /// applied; in MVP they're in-memory. Each entry records the
    /// Datasource, the list of referencing Jobs to drain, and the
    /// timestamp.
    draining_log: Vec<DrainingEvent>,
}

/// S29 redo: one Draining event. Triggered when a Datasource is
/// paused; lists the Jobs whose Tasks must migrate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainingEvent {
    pub tenant: u16,
    pub datasource: String,
    pub referencing_jobs: Vec<u32>,
    pub triggered_at_ms: u64,
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
            health: HashMap::new(),
            referencing_jobs: HashMap::new(),
            draining_log: Vec::new(),
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
    /// Active. S29 redo: also records a Draining event listing
    /// all referencing Jobs, so the control plane can migrate
    /// those Tasks. The event is appended to `draining_log` and
    /// also returned via [`Self::take_draining_events`] for the
    /// caller to dispatch.
    pub fn pause(
        &mut self,
        tenant: u16,
        name: &str,
    ) -> DatasourceResult<Option<DrainingEvent>> {
        let event = self.collect_draining_event(tenant, name)?;
        self.set_status(tenant, name, DatasourceStatus::Paused, "active")?;
        if let Some(ev) = event.clone() {
            self.draining_log.push(ev.clone());
            Ok(Some(ev))
        } else {
            Ok(None)
        }
    }

    /// S29 redo: build a Draining event WITHOUT mutating status.
    /// Returns `Some` if the Datasource has referencing Jobs, `None`
    /// if it has none. Used internally by `pause`; also exposed for
    /// the auto-pause path (`should_auto_pause` -> `pause`).
    fn collect_draining_event(
        &self,
        tenant: u16,
        name: &str,
    ) -> DatasourceResult<Option<DrainingEvent>> {
        let key = (tenant, name.to_string());
        if !self.by_key.contains_key(&key) {
            return Err(DatasourceError::NotFound {
                tenant,
                name: name.to_string(),
            });
        }
        let jobs: Vec<u32> = self
            .referencing_jobs
            .get(&key)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        if jobs.is_empty() {
            return Ok(None);
        }
        Ok(Some(DrainingEvent {
            tenant,
            datasource: name.to_string(),
            referencing_jobs: jobs,
            triggered_at_ms: now_ms(),
        }))
    }

    /// S29 redo: drain the accumulated Draining events. The caller
    /// (control plane dispatcher) consumes these and triggers
    /// `Op::StealTask` for each `(JobId, TaskId)` to migrate them.
    /// Returns the events in append order.
    pub fn take_draining_events(&mut self) -> Vec<DrainingEvent> {
        std::mem::take(&mut self.draining_log)
    }

    /// S29 redo: peek at the accumulated Draining events without
    /// removing them. Useful for `inspect`.
    pub fn draining_events(&self) -> &[DrainingEvent] {
        &self.draining_log
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
        // S31: also drop the health + referencing_jobs entries.
        self.health.remove(&key);
        self.referencing_jobs.remove(&key);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    // ---- S31: health probe + auto-pause + referencing jobs ----

    /// Get or create the [`DatasourceHealth`] for a Datasource. The
    /// health struct is created lazily on first access; the
    /// default thresholds are S31 spec defaults.
    pub fn health(&mut self, tenant: u16, name: &str) -> DatasourceResult<Arc<DatasourceHealth>> {
        let key = (tenant, name.to_string());
        if !self.by_key.contains_key(&key) {
            return Err(DatasourceError::NotFound {
                tenant,
                name: name.to_string(),
            });
        }
        let health = self
            .health
            .entry(key)
            .or_insert_with(|| Arc::new(DatasourceHealth::new()))
            .clone();
        Ok(health)
    }

    /// Record a successful probe. Resets the consecutive-failure
    /// counter and updates the last_success timestamp.
    pub fn record_probe_success(&mut self, tenant: u16, name: &str) -> DatasourceResult<()> {
        let h = self.health(tenant, name)?;
        h.record_success();
        Ok(())
    }

    /// Record a failed probe. Increments the consecutive-failure
    /// counter, updates the last_failure timestamp + error message.
    /// Returns the health snapshot; the caller checks
    /// `should_auto_pause` to trigger Draining.
    pub fn record_probe_failure(
        &mut self,
        tenant: u16,
        name: &str,
        error: String,
    ) -> DatasourceResult<DatasourceHealthSnapshot> {
        let h = self.health(tenant, name)?;
        h.record_failure(error);
        Ok(h.snapshot())
    }

    /// Register a Job as referencing this Datasource. Called by
    /// the Deployer when a Pipeline using this Datasource is
    /// submitted. Idempotent.
    pub fn add_referencing_job(
        &mut self,
        tenant: u16,
        name: &str,
        job_id: u32,
    ) -> DatasourceResult<()> {
        let key = (tenant, name.to_string());
        if !self.by_key.contains_key(&key) {
            return Err(DatasourceError::NotFound {
                tenant,
                name: name.to_string(),
            });
        }
        self.referencing_jobs
            .entry(key)
            .or_default()
            .insert(job_id);
        Ok(())
    }

    /// Remove a Job from the referencing set. Called when a
    /// Pipeline completes or the Datasource is paused (Draining
    /// eventually drains all referencing Jobs).
    pub fn remove_referencing_job(
        &mut self,
        tenant: u16,
        name: &str,
        job_id: u32,
    ) -> DatasourceResult<()> {
        let key = (tenant, name.to_string());
        if let Some(set) = self.referencing_jobs.get_mut(&key) {
            set.remove(&job_id);
        }
        Ok(())
    }

    /// Number of Jobs currently referencing this Datasource.
    pub fn referencing_job_count(&self, tenant: u16, name: &str) -> usize {
        self.referencing_jobs
            .get(&(tenant, name.to_string()))
            .map(|s| s.len())
            .unwrap_or(0)
    }
}

/// S31 health probe state. Per-Datasource. Thread-safe: the
/// background probe task writes via `record_success` /
/// `record_failure`; the CLI's `bee datasource inspect` reads via
/// `snapshot`. The struct holds no async state — the probe is
/// driven externally.
#[derive(Debug)]
pub struct DatasourceHealth {
    pub connection_success_total: AtomicU64,
    pub connection_failure_total: AtomicU64,
    pub consecutive_failures: AtomicU32,
    pub last_success_at_ms: AtomicU64,
    pub last_failure_at_ms: AtomicU64,
    /// Most recent error message. `Mutex<Option<String>>` because
    /// the error itself is a String (variable size).
    pub error_message_recent: Mutex<Option<String>>,
    /// S31 spec default: 10 consecutive failures triggers
    /// auto-pause. Configurable per-Datasource in 1.x.
    pub auto_pause_threshold: u32,
}

impl Default for DatasourceHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl DatasourceHealth {
    pub fn new() -> Self {
        Self {
            connection_success_total: AtomicU64::new(0),
            connection_failure_total: AtomicU64::new(0),
            consecutive_failures: AtomicU32::new(0),
            last_success_at_ms: AtomicU64::new(0),
            last_failure_at_ms: AtomicU64::new(0),
            error_message_recent: Mutex::new(None),
            auto_pause_threshold: 10,
        }
    }

    pub fn with_threshold(mut self, threshold: u32) -> Self {
        self.auto_pause_threshold = threshold;
        self
    }

    pub fn record_success(&self) {
        self.connection_success_total
            .fetch_add(1, Ordering::Relaxed);
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.last_success_at_ms.store(now_ms(), Ordering::Relaxed);
    }

    pub fn record_failure(&self, error: String) {
        self.connection_failure_total
            .fetch_add(1, Ordering::Relaxed);
        self.consecutive_failures
            .fetch_add(1, Ordering::Relaxed);
        self.last_failure_at_ms.store(now_ms(), Ordering::Relaxed);
        *self
            .error_message_recent
            .lock()
            .expect("poisoned") = Some(error);
    }

    /// True if the consecutive-failure counter has crossed the
    /// auto-pause threshold. The caller (background probe loop)
    /// calls `DatasourceRegistry::pause` when this returns true.
    pub fn should_auto_pause(&self) -> bool {
        self.consecutive_failures.load(Ordering::Relaxed) >= self.auto_pause_threshold
    }

    pub fn snapshot(&self) -> DatasourceHealthSnapshot {
        DatasourceHealthSnapshot {
            connection_success_total: self
                .connection_success_total
                .load(Ordering::Relaxed),
            connection_failure_total: self
                .connection_failure_total
                .load(Ordering::Relaxed),
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed),
            last_success_at_ms: self.last_success_at_ms.load(Ordering::Relaxed),
            last_failure_at_ms: self.last_failure_at_ms.load(Ordering::Relaxed),
            error_message_recent: self
                .error_message_recent
                .lock()
                .expect("poisoned")
                .clone(),
            auto_pause_threshold: self.auto_pause_threshold,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Plain-data snapshot of a [`DatasourceHealth`] for printing /
/// serialization. Acquired via `DatasourceHealth::snapshot()`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DatasourceHealthSnapshot {
    pub connection_success_total: u64,
    pub connection_failure_total: u64,
    pub consecutive_failures: u32,
    pub last_success_at_ms: u64,
    pub last_failure_at_ms: u64,
    pub error_message_recent: Option<String>,
    pub auto_pause_threshold: u32,
}

/// One-stop view returned by `DatasourceRegistry::inspect`. S31
/// acceptance: `bee datasource inspect binance` shows Producer
/// Node, plugin_id, version, health metrics, referencing Job count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasourceInspection {
    pub datasource: Datasource,
    pub health: DatasourceHealthSnapshot,
    pub producer_node: Option<u32>,
    pub referencing_job_count: usize,
}

impl DatasourceRegistry {
    /// Convenience: build the inspection view for a Datasource.
    /// Returns `None` if the Datasource doesn't exist.
    pub fn inspect(&self, tenant: u16, name: &str) -> Option<DatasourceInspection> {
        let ds = self.get(tenant, name)?.clone();
        let key = (tenant, name.to_string());
        let health = self
            .health
            .get(&key)
            .map(|h| h.snapshot())
            .unwrap_or_else(DatasourceHealthSnapshot::default);
        let job_count = self.referencing_job_count(tenant, name);
        Some(DatasourceInspection {
            producer_node: ds.owner_node,
            referencing_job_count: job_count,
            datasource: ds,
            health,
        })
    }

    /// S29 redo: lookup a Datasource and produce the
    /// `DatasourceInfo` shape the SQL preprocessor needs (no
    /// `PluginId`, no `config`, no timestamps). Returns `None` if
    /// the Datasource doesn't exist.
    pub fn lookup_for_preprocess(
        &self,
        tenant: u16,
        name: &str,
    ) -> Option<bee_dsl_sql::preprocess::DatasourceInfo> {
        self.get(tenant, name).map(|ds| {
            bee_dsl_sql::preprocess::DatasourceInfo {
                name: ds.name.clone(),
                tenant: ds.tenant,
                adapter: ds.adapter.clone(),
                version_spec: ds.version_spec.clone(),
            }
        })
    }

    /// S29 redo: list Datasources that reference a given Plugin by
    /// its `PluginId`. Used by the pause cascade to find all
    /// Datasources backed by the same Plugin.
    pub fn datasources_for_plugin(
        &self,
        plugin_id: &bee_plugin_sdk::PluginId,
    ) -> Vec<(u16, String)> {
        self.by_key
            .iter()
            .filter(|(_, ds)| &ds.plugin_id == plugin_id)
            .map(|((t, n), _)| (*t, n.clone()))
            .collect()
    }
}

/// S29 redo: implement the preprocessor's `DatasourceLookup` trait
/// for `DatasourceRegistry`.
impl bee_dsl_sql::preprocess::DatasourceLookup for DatasourceRegistry {
    fn lookup(
        &self,
        tenant: u16,
        name: &str,
    ) -> Option<bee_dsl_sql::preprocess::DatasourceInfo> {
        self.lookup_for_preprocess(tenant, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_plugin_sdk::compute_plugin_id;
    use std::sync::atomic::Ordering;

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

    // ---- S31 health + auto-pause + referencing jobs ----

    #[test]
    fn health_default_threshold_is_10_per_spec() {
        let h = DatasourceHealth::new();
        assert_eq!(h.auto_pause_threshold, 10);
    }

    #[test]
    fn record_success_resets_consecutive_failures() {
        let h = DatasourceHealth::new();
        h.record_failure("net err".into());
        h.record_failure("net err".into());
        assert_eq!(h.consecutive_failures.load(Ordering::Relaxed), 2);
        h.record_success();
        assert_eq!(h.consecutive_failures.load(Ordering::Relaxed), 0);
        assert_eq!(h.connection_success_total.load(Ordering::Relaxed), 1);
        assert_eq!(h.connection_failure_total.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn should_auto_pause_triggers_at_threshold() {
        // S31 acceptance: 10 consecutive failures triggers auto-pause.
        let h = DatasourceHealth::new();
        for _ in 0..9 {
            h.record_failure("net err".into());
        }
        assert!(!h.should_auto_pause(), "9 failures should NOT trigger");
        h.record_failure("net err".into());
        assert!(h.should_auto_pause(), "10 failures MUST trigger");
        // Any subsequent failure still triggers
        h.record_failure("net err".into());
        assert!(h.should_auto_pause());
    }

    #[test]
    fn success_resets_threshold_counting() {
        // If a single success arrives in the middle of the streak,
        // the counter resets and the auto-pause threshold needs to
        // be crossed again.
        let h = DatasourceHealth::new();
        for _ in 0..9 {
            h.record_failure("net err".into());
        }
        h.record_success();
        for _ in 0..9 {
            h.record_failure("net err".into());
        }
        assert!(!h.should_auto_pause(), "should reset after success");
        h.record_failure("net err".into());
        assert!(h.should_auto_pause());
    }

    #[test]
    fn error_message_recent_updates_on_failure() {
        let h = DatasourceHealth::new();
        h.record_failure("connection refused".into());
        assert_eq!(
            h.error_message_recent.lock().unwrap().as_deref(),
            Some("connection refused")
        );
        h.record_failure("timeout".into());
        assert_eq!(
            h.error_message_recent.lock().unwrap().as_deref(),
            Some("timeout")
        );
    }

    #[test]
    fn snapshot_is_plain_data() {
        let h = DatasourceHealth::new();
        h.record_failure("err".into());
        let s = h.snapshot();
        assert_eq!(s.connection_failure_total, 1);
        assert_eq!(s.consecutive_failures, 1);
        assert_eq!(s.error_message_recent.as_deref(), Some("err"));
        assert!(s.last_failure_at_ms > 0);
    }

    #[test]
    fn add_referencing_job_tracks_count() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("binance", 0)).unwrap();
        r.add_referencing_job(0, "binance", 100).unwrap();
        r.add_referencing_job(0, "binance", 100).unwrap(); // idempotent
        r.add_referencing_job(0, "binance", 101).unwrap();
        assert_eq!(r.referencing_job_count(0, "binance"), 2);
    }

    #[test]
    fn remove_referencing_job_decrements() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("binance", 0)).unwrap();
        r.add_referencing_job(0, "binance", 100).unwrap();
        r.add_referencing_job(0, "binance", 101).unwrap();
        r.remove_referencing_job(0, "binance", 100).unwrap();
        assert_eq!(r.referencing_job_count(0, "binance"), 1);
    }

    #[test]
    fn inspect_returns_datasource_health_and_job_count() {
        // S31 acceptance: bee datasource inspect shows Producer
        // Node, plugin_id, version, health metrics, referencing Job count.
        let mut r = DatasourceRegistry::new();
        let ds = test_ds("binance", 0);
        r.create(ds).unwrap();
        r.add_referencing_job(0, "binance", 100).unwrap();
        r.add_referencing_job(0, "binance", 101).unwrap();
        r.record_probe_success(0, "binance").unwrap();
        r.record_probe_failure(0, "binance", "boom".into()).unwrap();

        let i = r.inspect(0, "binance").expect("inspect");
        assert_eq!(i.datasource.name, "binance");
        assert_eq!(i.health.connection_success_total, 1);
        assert_eq!(i.health.connection_failure_total, 1);
        assert_eq!(i.health.consecutive_failures, 1);
        assert_eq!(i.referencing_job_count, 2);
    }

    #[test]
    fn record_probe_failure_returns_snapshot() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("binance", 0)).unwrap();
        let s = r
            .record_probe_failure(0, "binance", "boom".into())
            .unwrap();
        assert_eq!(s.consecutive_failures, 1);
        assert_eq!(s.error_message_recent.as_deref(), Some("boom"));
    }

    #[test]
    fn health_for_unknown_datasource_errors() {
        let mut r = DatasourceRegistry::new();
        let err = r.health(0, "absent").unwrap_err();
        assert!(matches!(err, DatasourceError::NotFound { .. }));
    }

    #[test]
    fn delete_drops_health_and_referencing_jobs() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("binance", 0)).unwrap();
        r.add_referencing_job(0, "binance", 100).unwrap();
        // Touch the health so it materializes
        let _ = r.record_probe_success(0, "binance").unwrap();
        r.delete(0, "binance").unwrap();
        // After delete, health and referencing_jobs are gone.
        assert_eq!(r.referencing_job_count(0, "binance"), 0);
        // And a re-create starts with a fresh health state.
        r.create(test_ds("binance", 0)).unwrap();
        let i = r.inspect(0, "binance").unwrap();
        assert_eq!(i.health.connection_success_total, 0);
        assert_eq!(i.health.consecutive_failures, 0);
    }

    // ---- S29 redo: Draining on pause ----

    #[test]
    fn pause_records_draining_event_with_referencing_jobs() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("binance", 0)).unwrap();
        r.add_referencing_job(0, "binance", 100).unwrap();
        r.add_referencing_job(0, "binance", 101).unwrap();
        r.add_referencing_job(0, "binance", 102).unwrap();
        let ev = r.pause(0, "binance").unwrap().expect("event");
        assert_eq!(ev.tenant, 0);
        assert_eq!(ev.datasource, "binance");
        assert_eq!(ev.referencing_jobs.len(), 3);
        assert!(ev.triggered_at_ms > 0);
        // Event is also in the log.
        assert_eq!(r.draining_events().len(), 1);
        assert_eq!(r.draining_events()[0], ev);
    }

    #[test]
    fn pause_with_no_referencing_jobs_emits_no_event() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("binance", 0)).unwrap();
        let ev = r.pause(0, "binance").unwrap();
        assert!(ev.is_none(), "no Jobs = no event");
        assert!(r.draining_events().is_empty());
    }

    #[test]
    fn take_draining_events_clears_the_log() {
        let mut r = DatasourceRegistry::new();
        r.create(test_ds("a", 0)).unwrap();
        r.create(test_ds("b", 0)).unwrap();
        r.add_referencing_job(0, "a", 1).unwrap();
        r.add_referencing_job(0, "b", 2).unwrap();
        r.pause(0, "a").unwrap();
        r.pause(0, "b").unwrap();
        assert_eq!(r.draining_events().len(), 2);
        let drained = r.take_draining_events();
        assert_eq!(drained.len(), 2);
        assert!(r.draining_events().is_empty());
    }

    // ---- S29 redo: preprocessor trait impl ----

    #[test]
    fn registry_implements_datasource_lookup() {
        let r = DatasourceRegistry::new();
        let mut r = r;
        r.create(test_ds("binance", 0)).unwrap();
        let info = r
            .lookup_for_preprocess(0, "binance")
            .expect("found");
        assert_eq!(info.name, "binance");
        assert_eq!(info.tenant, 0);
        assert_eq!(info.adapter, "binance");
        // PluginManager integration test in the runtime.
    }

    #[test]
    fn registry_lookup_for_preprocess_returns_none_for_missing() {
        let r = DatasourceRegistry::new();
        assert!(r.lookup_for_preprocess(0, "absent").is_none());
    }
}
