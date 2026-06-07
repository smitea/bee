//! `SecretStore` — credential isolation (S30, ADR-0010).
//!
//! Credentials (API keys, OAuth tokens, DB passwords) live in a
//! dedicated SecretStore, not in Datasource `config`. The
//! Datasource's `config` references secrets by ID; the Plugin reads
//! the actual value at runtime via the `BeeHost` API.
//!
//! ## S30 MVP scope
//! - [`SecretStore`] trait with `get` / `put` / `delete` / `list`
//! - [`InMemorySecretStore`] impl (HashMap-backed; per MVP)
//! - Tenant-scoped: `tenant: u16` namespace; the MVP default is
//!   tenant 0 (global), per ADR-0010
//! - Values are opaque `Vec<u8>` (the Plugin decides the encoding)
//! - `list` returns IDs only (not values), per S30 acceptance
//!
//! ## S30+ follow-ups
//! - Raft-replicated KV persistence (the existing KV SM is the
//!   substrate; the wiring is one `Op::SecretPut` away)
//! - Encryption-at-rest (Raft log encryption / Vault / AWS SM)
//! - The `BeeHostV1::secret_get` C function pointer for Plugin
//!   access (the trait hook is in place; the FFI symbol is wired
//!   in the S19+ follow-up)
//! - `bee secret put/get/list/delete` CLI in the bee binary

use std::collections::{BTreeMap, HashMap};

/// Errors from the SecretStore. The MVP collapses all failures
/// into a single string; a 1.x refactor can split into typed
/// variants (NotFound, AlreadyExists, PermissionDenied, etc.).
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret `{tenant}/{secret_id}` already exists")]
    AlreadyExists { tenant: u16, secret_id: String },
    #[error("invalid secret id `{0}` (empty)")]
    InvalidId(String),
    #[error("store: {0}")]
    Store(String),
}

pub type SecretResult<T> = std::result::Result<T, SecretError>;

/// SecretStore contract. The MVP implementation is
/// [`InMemorySecretStore`]; 1.x wires a Raft-replicated KV impl
/// and a Vault / AWS-SM backend.
pub trait SecretStore: Send + Sync {
    /// Read a secret's bytes. Returns `Ok(None)` if the secret
    /// doesn't exist.
    fn get(&self, tenant: u16, secret_id: &str) -> SecretResult<Option<Vec<u8>>>;
    /// Store a secret's bytes. Errors if the secret already
    /// exists — callers should `delete` first to overwrite.
    fn put(&self, tenant: u16, secret_id: &str, value: Vec<u8>) -> SecretResult<()>;
    /// Delete a secret. Errors if the secret doesn't exist.
    fn delete(&self, tenant: u16, secret_id: &str) -> SecretResult<()>;
    /// List secret IDs in the tenant. The MVP returns IDs only
    /// (per S30 acceptance: "shows secret IDs only, not values").
    /// The list is sorted lexicographically for deterministic
    /// output (`bee secret list`).
    fn list(&self, tenant: u16) -> Vec<String>;
    /// Number of stored secrets in the tenant. Used by tests and
    /// by the future `bee secret count` admin surface.
    fn count(&self, tenant: u16) -> usize;
}

/// In-memory MVP SecretStore. Thread-safe via a `parking_lot`-style
/// Mutex (we use `std::sync::Mutex` to keep zero-runtime-deps). The
/// S30+ Raft impl will replace the inner HashMap with a KV SM
/// round-trip.
pub struct InMemorySecretStore {
    store: std::sync::Mutex<HashMap<(u16, String), Vec<u8>>>,
}

impl Default for InMemorySecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self {
            store: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

impl SecretStore for InMemorySecretStore {
    fn get(&self, tenant: u16, secret_id: &str) -> SecretResult<Option<Vec<u8>>> {
        if secret_id.is_empty() {
            return Err(SecretError::InvalidId(secret_id.to_string()));
        }
        let store = self.store.lock().expect("poisoned");
        Ok(store.get(&(tenant, secret_id.to_string())).cloned())
    }

    fn put(&self, tenant: u16, secret_id: &str, value: Vec<u8>) -> SecretResult<()> {
        if secret_id.is_empty() {
            return Err(SecretError::InvalidId(secret_id.to_string()));
        }
        let key = (tenant, secret_id.to_string());
        let mut store = self.store.lock().expect("poisoned");
        if store.contains_key(&key) {
            return Err(SecretError::AlreadyExists {
                tenant,
                secret_id: secret_id.to_string(),
            });
        }
        store.insert(key, value);
        Ok(())
    }

    fn delete(&self, tenant: u16, secret_id: &str) -> SecretResult<()> {
        if secret_id.is_empty() {
            return Err(SecretError::InvalidId(secret_id.to_string()));
        }
        let key = (tenant, secret_id.to_string());
        let mut store = self.store.lock().expect("poisoned");
        store
            .remove(&key)
            .ok_or_else(|| SecretError::Store(format!("secret {tenant}/{secret_id} not found")))?;
        Ok(())
    }

    fn list(&self, tenant: u16) -> Vec<String> {
        let store = self.store.lock().expect("poisoned");
        // BTreeMap for deterministic order
        let mut sorted: BTreeMap<&String, ()> = BTreeMap::new();
        for ((t, id), _) in store.iter() {
            if *t == tenant {
                sorted.insert(id, ());
            }
        }
        sorted.keys().map(|s| s.to_string()).collect()
    }

    fn count(&self, tenant: u16) -> usize {
        let store = self.store.lock().expect("poisoned");
        store.keys().filter(|(t, _)| *t == tenant).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_starts_empty() {
        let s = InMemorySecretStore::new();
        assert_eq!(s.list(0).len(), 0);
        assert_eq!(s.count(0), 0);
    }

    #[test]
    fn put_then_get_round_trip() {
        let s = InMemorySecretStore::new();
        s.put(0, "api_key", b"super-secret".to_vec()).unwrap();
        let v = s.get(0, "api_key").unwrap().expect("present");
        assert_eq!(v, b"super-secret");
    }

    #[test]
    fn get_missing_secret_returns_none() {
        let s = InMemorySecretStore::new();
        let v = s.get(0, "absent").unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn duplicate_put_errors() {
        let s = InMemorySecretStore::new();
        s.put(0, "k", b"v1".to_vec()).unwrap();
        let err = s.put(0, "k", b"v2".to_vec()).unwrap_err();
        assert!(matches!(err, SecretError::AlreadyExists { .. }));
    }

    #[test]
    fn delete_removes_secret() {
        let s = InMemorySecretStore::new();
        s.put(0, "k", b"v".to_vec()).unwrap();
        s.delete(0, "k").unwrap();
        assert!(s.get(0, "k").unwrap().is_none());
    }

    #[test]
    fn delete_missing_errors() {
        let s = InMemorySecretStore::new();
        let err = s.delete(0, "absent").unwrap_err();
        assert!(matches!(err, SecretError::Store(_)));
    }

    #[test]
    fn list_returns_ids_sorted_per_tenant() {
        let s = InMemorySecretStore::new();
        for id in ["zebra", "alpha", "mango"] {
            s.put(0, id, b"v".to_vec()).unwrap();
        }
        s.put(1, "tenant1-only", b"v".to_vec()).unwrap();
        assert_eq!(s.list(0), vec!["alpha", "mango", "zebra"]);
        assert_eq!(s.list(1), vec!["tenant1-only"]);
    }

    #[test]
    fn secrets_are_tenant_scoped() {
        // S30 acceptance: secrets are scoped per tenant.
        let s = InMemorySecretStore::new();
        s.put(0, "global-api-key", b"g".to_vec()).unwrap();
        s.put(7, "tenant-7-api-key", b"7".to_vec()).unwrap();
        assert_eq!(s.list(0), vec!["global-api-key"]);
        assert_eq!(s.list(7), vec!["tenant-7-api-key"]);
        // No cross-tenant leak
        assert!(s.get(7, "global-api-key").unwrap().is_none());
        assert!(s.get(0, "tenant-7-api-key").unwrap().is_none());
    }

    #[test]
    fn empty_id_rejected_for_put() {
        let s = InMemorySecretStore::new();
        let err = s.put(0, "", b"v".to_vec()).unwrap_err();
        assert!(matches!(err, SecretError::InvalidId(_)));
    }

    #[test]
    fn empty_id_rejected_for_get() {
        let s = InMemorySecretStore::new();
        let err = s.get(0, "").unwrap_err();
        assert!(matches!(err, SecretError::InvalidId(_)));
    }

    #[test]
    fn trait_object_dispatch_works() {
        // The CLI / tests use &dyn SecretStore; verify the trait
        // is dyn-safe (no Self: Sized bounds on any method).
        let store: Box<dyn SecretStore> = Box::new(InMemorySecretStore::new());
        store.put(0, "k", b"v".to_vec()).unwrap();
        let v = store.get(0, "k").unwrap().expect("present");
        assert_eq!(v, b"v");
    }
}
