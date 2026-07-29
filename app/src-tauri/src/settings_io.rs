use std::fs;
use std::path::Path;

use crate::db::{self, Database};

pub fn import_legacy_addr(db: &Database, json_path: &Path) -> Result<(), String> {
    let conn = db.lock()?;
    if db::settings::get(&conn, "addr")?.is_some() {
        return Ok(());
    }
    let content = match fs::read_to_string(json_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("read {}: {e}", json_path.display())),
    };
    let val: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if let Some(addr) = val.get("addr").and_then(|v| v.as_str()) {
        db::settings::put(&conn, "addr", addr)?;
    }
    let _ = fs::remove_file(json_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn run<F: FnOnce(&Database, &std::path::Path)>(f: F) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("bee-client.sqlite");
        let json_path = dir.path().join("settings.json");
        let db = Database::open(&db_path).unwrap();
        f(&db, &json_path);
    }

    #[test]
    fn imports_addr_when_table_empty_and_file_present() {
        run(|db, json_path| {
            fs::write(json_path, r#"{"addr":"10.0.0.1:10001"}"#).unwrap();
            import_legacy_addr(db, json_path).unwrap();
            let conn = db.lock().unwrap();
            assert_eq!(
                db::settings::get(&conn, "addr").unwrap().as_deref(),
                Some("10.0.0.1:10001")
            );
            assert!(!json_path.exists());
        });
    }

    #[test]
    fn noop_when_addr_already_in_db() {
        run(|db, json_path| {
            fs::write(json_path, r#"{"addr":"10.0.0.1:10001"}"#).unwrap();
            {
                let conn = db.lock().unwrap();
                db::settings::put(&conn, "addr", "127.0.0.1:9999").unwrap();
            }
            import_legacy_addr(db, json_path).unwrap();
            let conn = db.lock().unwrap();
            assert_eq!(
                db::settings::get(&conn, "addr").unwrap().as_deref(),
                Some("127.0.0.1:9999")
            );
        });
    }

    #[test]
    fn noop_when_json_missing() {
        run(|db, json_path| {
            import_legacy_addr(db, json_path).unwrap();
            let conn = db.lock().unwrap();
            assert_eq!(db::settings::get(&conn, "addr").unwrap(), None);
        });
    }

    #[test]
    fn noop_when_json_malformed() {
        run(|db, json_path| {
            fs::write(json_path, "{not json").unwrap();
            import_legacy_addr(db, json_path).unwrap();
            let conn = db.lock().unwrap();
            assert_eq!(db::settings::get(&conn, "addr").unwrap(), None);
        });
    }
}
