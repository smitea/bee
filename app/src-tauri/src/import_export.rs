use std::fs;
use std::path::Path;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use rand::RngCore;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::{self, Database};

const ENVELOPE_VERSION: u32 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u32,
    pub salt: String,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApplicationExport {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub dashboards: Vec<String>,
    pub pipelines: Vec<String>,
    pub datasources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationRef {
    pub name: String,
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub created: Vec<ApplicationRef>,
    pub skipped: Vec<String>,
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], String> {
    let params = Params::new(19_456, 2, 1, Some(KEY_LEN))
        .map_err(|e| format!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut buf = [0u8; KEY_LEN];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut buf)
        .map_err(|e| format!("argon2 derive: {e}"))?;
    Ok(buf)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

fn collect_application(conn: &Connection, name: &str) -> Result<ApplicationExport, String> {
    let app = db::applications::list(conn)?
        .into_iter()
        .find(|a| a.name == name)
        .ok_or_else(|| format!("export: application '{name}' not found"))?;
    let resources = db::applications::resources_for(conn, app.id)?;
    let mut dashboards = Vec::new();
    let mut pipelines = Vec::new();
    let mut datasources = Vec::new();
    for (kind, ref_id) in resources {
        let rid = ref_id.unwrap_or_default();
        match kind.as_str() {
            "dashboard" => dashboards.push(rid),
            "pipeline" => pipelines.push(rid),
            "datasource" => datasources.push(rid),
            _ => {}
        }
    }
    Ok(ApplicationExport {
        id: app.id,
        name: app.name,
        enabled: app.enabled,
        dashboards,
        pipelines,
        datasources,
    })
}

fn encrypt_envelope(app: &ApplicationExport, passphrase: &str) -> Result<Envelope, String> {
    let salt = random_bytes::<SALT_LEN>();
    let nonce_bytes = random_bytes::<NONCE_LEN>();
    let key_bytes = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    let plaintext = serde_json::to_vec(app).map_err(|e| format!("ser: {e}"))?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|e| format!("encrypt: {e}"))?;
    Ok(Envelope {
        version: ENVELOPE_VERSION,
        salt: b64().encode(salt),
        nonce: b64().encode(nonce_bytes),
        ciphertext: b64().encode(ct),
    })
}

fn decrypt_envelope(env: &Envelope, passphrase: &str) -> Result<ApplicationExport, String> {
    if env.version != ENVELOPE_VERSION {
        return Err(format!(
            "envelope: unsupported version {} (expected {})",
            env.version, ENVELOPE_VERSION
        ));
    }
    let salt = b64()
        .decode(env.salt.as_bytes())
        .map_err(|e| format!("envelope: bad salt: {e}"))?;
    let nonce_bytes = b64()
        .decode(env.nonce.as_bytes())
        .map_err(|e| format!("envelope: bad nonce: {e}"))?;
    let ciphertext = b64()
        .decode(env.ciphertext.as_bytes())
        .map_err(|e| format!("envelope: bad ciphertext: {e}"))?;
    let key_bytes = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
        .map_err(|_| "envelope: decryption failed (wrong passphrase or tampered)".to_string())?;
    serde_json::from_slice(&plaintext).map_err(|e| format!("envelope: bad plaintext: {e}"))
}

pub fn export_application(
    db: &Database,
    name: &str,
    passphrase: &str,
    out_path: &Path,
) -> Result<(), String> {
    if passphrase.is_empty() {
        return Err("export: passphrase must not be empty".into());
    }
    let app = {
        let conn = db.lock().map_err(|e| format!("export.lock: {e}"))?;
        collect_application(&conn, name)?
    };
    let envelope = encrypt_envelope(&app, passphrase)?;
    let json = serde_json::to_string_pretty(&envelope)
        .map_err(|e| format!("ser envelope: {e}"))?;
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
    }
    fs::write(out_path, json).map_err(|e| format!("write {}: {e}", out_path.display()))?;
    Ok(())
}

pub fn import_application(
    db: &Database,
    file_path: &Path,
    passphrase: &str,
) -> Result<ImportReport, String> {
    if passphrase.is_empty() {
        return Err("import: passphrase must not be empty".into());
    }
    let raw = fs::read_to_string(file_path)
        .map_err(|e| format!("read {}: {e}", file_path.display()))?;
    let envelope: Envelope = serde_json::from_str(&raw)
        .map_err(|e| format!("envelope: invalid json: {e}"))?;
    let app = decrypt_envelope(&envelope, passphrase)?;

    let mut report = ImportReport::default();
    db.apply_migrations()?;
    let conn = db.lock().map_err(|e| format!("import.lock: {e}"))?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("import: begin tx: {e}"))?;
    if db::applications::name_taken(&conn, &app.name).unwrap_or(true) {
        report.skipped.push(app.name.clone());
        tx.commit().map_err(|e| format!("commit: {e}"))?;
        return Ok(report);
    }
    let created = db::applications::create(&conn, &app.name)?;
    if !app.enabled {
        db::applications::set_enabled(&conn, created.id, false)?;
    }
    for d in &app.dashboards {
        db::applications::add_resource(&conn, created.id, "dashboard", Some(d))?;
    }
    for p in &app.pipelines {
        db::applications::add_resource(&conn, created.id, "pipeline", Some(p))?;
    }
    for ds in &app.datasources {
        db::applications::add_resource(&conn, created.id, "datasource", Some(ds))?;
    }
    report.created.push(ApplicationRef { name: created.name });
    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::fs;
    use tempfile::tempdir;

    fn run<F: FnOnce(&Database, &std::path::Path, &std::path::Path)>(f: F) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("bee-client.sqlite");
        let out_path = dir.path().join("app.bapp");
        let db = Database::open(&db_path).unwrap();
        f(&db, &out_path, dir.path());
    }

    #[test]
    fn round_trip_export_then_import_recovers_application_name_and_enabled() {
        run(|db, out_path, _| {
            {
                let conn = db.lock().unwrap();
                crate::db::applications::create(&conn, "alpha").unwrap();
            }
            let passphrase = "correct horse battery staple";
            export_application(db, "alpha", passphrase, out_path).unwrap();
            assert!(out_path.exists());

            {
                let conn = db.lock().unwrap();
                crate::db::applications::delete(&conn, 1).unwrap();
            }

            let report = import_application(db, out_path, passphrase).unwrap();
            assert_eq!(report.created.len(), 1);
            assert_eq!(report.created[0].name, "alpha");
            assert!(report.skipped.is_empty());

            let conn = db.lock().unwrap();
            let all = crate::db::applications::list(&conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].name, "alpha");
            assert!(all[0].enabled);
        });
    }

    #[test]
    fn wrong_passphrase_returns_error_and_does_not_modify_db() {
        run(|db, out_path, _| {
            {
                let conn = db.lock().unwrap();
                crate::db::applications::create(&conn, "alpha").unwrap();
            }
            export_application(db, "alpha", "right", out_path).unwrap();
            {
                let conn = db.lock().unwrap();
                crate::db::applications::delete(&conn, 1).unwrap();
            }
            let result = import_application(db, out_path, "wrong");
            assert!(result.is_err(), "wrong passphrase must error");

            let conn = db.lock().unwrap();
            let all = crate::db::applications::list(&conn).unwrap();
            assert!(all.is_empty());
        });
    }

    #[test]
    fn tampered_ciphertext_returns_error() {
        run(|db, out_path, _| {
            {
                let conn = db.lock().unwrap();
                crate::db::applications::create(&conn, "alpha").unwrap();
            }
            export_application(db, "alpha", "pass", out_path).unwrap();

            let mut envelope: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(out_path).unwrap()).unwrap();
            let ct = envelope["ciphertext"].as_str().unwrap().to_string();
            let mut bytes = b64().decode(ct.as_bytes()).unwrap();
            assert!(!bytes.is_empty());
            bytes[0] ^= 0xFF;
            envelope["ciphertext"] = serde_json::Value::String(b64().encode(&bytes));
            fs::write(out_path, serde_json::to_string(&envelope).unwrap()).unwrap();

            let result = import_application(db, out_path, "pass");
            assert!(result.is_err(), "tampered ciphertext must error");
        });
    }

    #[test]
    fn malformed_envelope_returns_error() {
        run(|db, out_path, _| {
            fs::write(out_path, "{not valid json").unwrap();
            let result = import_application(db, out_path, "pass");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("envelope"));
        });
    }

    #[test]
    fn missing_fields_returns_error() {
        run(|db, out_path, _| {
            fs::write(out_path, r#"{"version":1}"#).unwrap();
            let result = import_application(db, out_path, "pass");
            assert!(result.is_err());
        });
    }

    #[test]
    fn unknown_application_name_returns_error_on_export() {
        run(|db, out_path, _| {
            let result = export_application(db, "nope", "pass", out_path);
            assert!(result.is_err());
            assert!(!out_path.exists());
        });
    }

    #[test]
    fn duplicate_import_skips_existing_application() {
        run(|db, out_path, _| {
            {
                let conn = db.lock().unwrap();
                crate::db::applications::create(&conn, "alpha").unwrap();
            }
            export_application(db, "alpha", "pass", out_path).unwrap();
            let report = import_application(db, out_path, "pass").unwrap();
            assert!(report.created.is_empty());
            assert_eq!(report.skipped.len(), 1);
            assert_eq!(report.skipped[0], "alpha");
            let conn = db.lock().unwrap();
            let all = crate::db::applications::list(&conn).unwrap();
            assert_eq!(all.len(), 1);
        });
    }

    #[test]
    fn empty_passphrase_is_rejected() {
        run(|db, out_path, _| {
            assert!(export_application(db, "alpha", "", out_path).is_err());
        });
    }

    #[test]
    fn import_is_atomic_when_payload_invalid() {
        run(|db, out_path, _| {
            fs::write(out_path, b"\x00\x01\x02").unwrap();
            let _ = import_application(db, out_path, "pass");

            let conn = db.lock().unwrap();
            let all = crate::db::applications::list(&conn).unwrap();
            assert!(all.is_empty(), "failed import must not leave partial state");
        });
    }
}