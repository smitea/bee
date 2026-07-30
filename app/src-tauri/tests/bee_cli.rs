use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use app_lib::db;

fn run_cli(database_path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bee-cli"))
        .env("BEE_CLIENT_DB", database_path)
        .args(args)
        .output()
        .expect("bee-cli must run")
}

fn success_stdout(database_path: &Path, args: &[&str]) -> String {
    let output = run_cli(database_path, args);
    assert!(
        output.status.success(),
        "bee-cli {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout must be UTF-8")
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar = path.as_os_str().to_os_string();
    sidecar.push(suffix);
    PathBuf::from(sidecar)
}

#[test]
fn lists_and_describes_sqlite_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database_path = dir.path().join("bee-client.sqlite");
    let database = db::Database::open(&database_path).expect("database");
    let (application_id, pipeline_id) = {
        let conn = database.lock().expect("lock");
        let application =
            db::applications::create_with_tenant(&conn, "analytics", 7).expect("application");
        let pipeline =
            db::pipelines::create(&conn, "prices", r#"{"source":"binance"}"#).expect("pipeline");
        db::datasources::create(
            &conn,
            "binance",
            "sample-kline",
            r#"{"base_url":"https://example.invalid"}"#,
            7,
        )
        .expect("datasource");
        db::plugin_settings::upsert(&conn, "sample-kline", true, "{}").expect("plugin setting");
        db::clusters::save(&conn, "CI", "10.0.0.8:9999", 7).expect("cluster profile");
        db::applications::record_disable_snapshot(&conn, application.id, "{}")
            .expect("disable snapshot");
        (application.id, pipeline.id)
    };
    drop(database);

    let applications = success_stdout(&database_path, &["list", "applications"]);
    assert!(applications.contains(&format!("{application_id}\tanalytics\ttrue\t7")));

    let pipelines = success_stdout(&database_path, &["list", "pipelines"]);
    assert!(pipelines.contains(&format!(
        "{pipeline_id}\tprices\t{{\"source\":\"binance\"}}"
    )));

    let datasources = success_stdout(&database_path, &["list", "datasources"]);
    assert!(datasources.contains("binance\tsample-kline\t7"));

    let plugins = success_stdout(&database_path, &["list", "plugins"]);
    assert!(plugins.contains("sample-kline\ttrue"));

    let application = success_stdout(
        &database_path,
        &["describe", "application", &application_id.to_string()],
    );
    assert!(application.contains("name\tanalytics"));
    assert!(application.contains("enabled\ttrue"));
    assert!(application.contains("tenant\t7"));
    assert!(application.contains("disable_snapshots\t1"));

    let connection = success_stdout(&database_path, &["describe", "connection"]);
    assert!(connection.contains("addr\t10.0.0.8:9999"));
    assert!(connection.contains("tenant\t7"));

    let migration_status = success_stdout(&database_path, &["migrate-status"]);
    let latest = db::MIGRATIONS.last().expect("migration").version;
    assert!(migration_status.contains(&format!("available\t{latest}")));
    assert!(migration_status.contains(&format!("applied\t{latest}")));
}

#[test]
fn reset_requires_force_and_deletes_database_and_sidecars_until_next_launch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database_path = dir.path().join("bee-client.sqlite");
    let database = db::Database::open(&database_path).expect("database");
    {
        let conn = database.lock().expect("lock");
        db::applications::create(&conn, "temporary").expect("application");
    }
    drop(database);

    let wal_path = sidecar(&database_path, "-wal");
    let shm_path = sidecar(&database_path, "-shm");
    std::fs::write(&wal_path, b"wal").expect("wal sidecar");
    std::fs::write(&shm_path, b"shm").expect("shm sidecar");

    let refused = run_cli(&database_path, &["reset"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--force"));
    assert!(database_path.exists());
    assert!(wal_path.exists());
    assert!(shm_path.exists());

    let reset = success_stdout(&database_path, &["reset", "--force"]);
    assert!(reset.contains("reset\t"));
    assert!(!database_path.exists());
    assert!(!wal_path.exists());
    assert!(!shm_path.exists());

    let applications = success_stdout(&database_path, &["list", "applications"]);
    assert!(applications.is_empty());
    assert!(database_path.exists());
    let fresh = db::Database::open(&database_path).expect("fresh database");
    assert_eq!(
        fresh.applied_versions().expect("versions").len(),
        db::MIGRATIONS.len()
    );
}

#[test]
fn default_database_path_matches_tauri_application_data_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut command = Command::new(env!("CARGO_BIN_EXE_bee-cli"));
    command.env_remove("BEE_CLIENT_DB").arg("migrate-status");

    #[cfg(target_os = "macos")]
    let expected = {
        command.env("HOME", dir.path());
        dir.path()
            .join("Library/Application Support/io.smitea.beeclient/bee-client.sqlite")
    };

    #[cfg(target_os = "windows")]
    let expected = {
        command.env("APPDATA", dir.path());
        dir.path().join("io.smitea.beeclient/bee-client.sqlite")
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let expected = {
        let data_dir = dir.path().join("data");
        command.env("XDG_DATA_HOME", &data_dir);
        data_dir.join("io.smitea.beeclient/bee-client.sqlite")
    };

    let output = command.output().expect("bee-cli must run");
    assert!(
        output.status.success(),
        "bee-cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        expected.exists(),
        "expected database at {}",
        expected.display()
    );
}
