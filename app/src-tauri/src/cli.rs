use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::db::{self, Database, MIGRATIONS};

const APP_IDENTIFIER: &str = "io.smitea.beeclient";
const DATABASE_FILE: &str = "bee-client.sqlite";

pub const USAGE: &str = "Usage:\n  bee-cli list <applications|pipelines|datasources|plugins>\n  bee-cli describe application <id>\n  bee-cli describe connection\n  bee-cli migrate-status\n  bee-cli reset --force";

enum CliCommand {
    ListApplications,
    ListPipelines,
    ListDatasources,
    ListPlugins,
    DescribeApplication(i64),
    DescribeConnection,
    MigrateStatus,
    Reset,
    Help,
}

pub fn database_path() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("BEE_CLIENT_DB").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    default_database_path()
}

#[cfg(target_os = "macos")]
fn default_database_path() -> Result<PathBuf, String> {
    home_dir().map(|home| {
        home.join("Library")
            .join("Application Support")
            .join(APP_IDENTIFIER)
            .join(DATABASE_FILE)
    })
}

#[cfg(target_os = "windows")]
fn default_database_path() -> Result<PathBuf, String> {
    env_path("APPDATA").map(|dir| dir.join(APP_IDENTIFIER).join(DATABASE_FILE))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn default_database_path() -> Result<PathBuf, String> {
    let data_dir = match env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => home_dir()?.join(".local").join("share"),
    };
    Ok(data_dir.join(APP_IDENTIFIER).join(DATABASE_FILE))
}

#[cfg(not(target_os = "windows"))]
fn home_dir() -> Result<PathBuf, String> {
    env_path("HOME")
}

fn env_path(name: &str) -> Result<PathBuf, String> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("cannot resolve Bee Client database path: {name} is not set"))
}

pub fn run(args: &[String], database_path: &Path, output: &mut dyn Write) -> Result<(), String> {
    let command = parse(args)?;
    match command {
        CliCommand::Reset => reset_database(database_path, output),
        CliCommand::Help => write_text(output, USAGE),
        command => {
            let database = Database::open(database_path)
                .map_err(|error| format!("open database {}: {error}", database_path.display()))?;
            execute(command, &database, output)
        }
    }
}

fn parse(args: &[String]) -> Result<CliCommand, String> {
    let parts = args.iter().map(String::as_str).collect::<Vec<_>>();
    match parts.as_slice() {
        ["list", "applications"] => Ok(CliCommand::ListApplications),
        ["list", "pipelines"] => Ok(CliCommand::ListPipelines),
        ["list", "datasources"] => Ok(CliCommand::ListDatasources),
        ["list", "plugins"] => Ok(CliCommand::ListPlugins),
        ["describe", "application", id] => id
            .parse::<i64>()
            .map(CliCommand::DescribeApplication)
            .map_err(|_| format!("invalid application id: {id}")),
        ["describe", "connection"] => Ok(CliCommand::DescribeConnection),
        ["migrate-status"] => Ok(CliCommand::MigrateStatus),
        ["reset", "--force"] => Ok(CliCommand::Reset),
        ["reset"] => Err("reset requires --force".to_string()),
        ["help"] | ["--help"] | ["-h"] => Ok(CliCommand::Help),
        _ => Err(USAGE.to_string()),
    }
}

fn execute(command: CliCommand, database: &Database, output: &mut dyn Write) -> Result<(), String> {
    if matches!(&command, CliCommand::MigrateStatus) {
        let available = MIGRATIONS
            .last()
            .map(|migration| migration.version)
            .unwrap_or(0);
        let applied = database.applied_versions()?.last().copied().unwrap_or(0);
        write_pair(output, "available", &available.to_string())?;
        write_pair(output, "applied", &applied.to_string())?;
        return Ok(());
    }
    let conn = database.lock()?;
    match command {
        CliCommand::ListApplications => {
            for application in db::applications::list(&conn)? {
                write_row(
                    output,
                    &[
                        application.id.to_string(),
                        field(&application.name),
                        application.enabled.to_string(),
                        application.tenant.to_string(),
                    ],
                )?;
            }
        }
        CliCommand::ListPipelines => {
            for pipeline in db::pipelines::list(&conn)? {
                write_row(
                    output,
                    &[
                        pipeline.id.to_string(),
                        field(&pipeline.name),
                        field(&pipeline.dag_json),
                        pipeline.updated_at.to_string(),
                    ],
                )?;
            }
        }
        CliCommand::ListDatasources => {
            for datasource in db::datasources::list(&conn)? {
                write_row(
                    output,
                    &[
                        field(&datasource.name),
                        field(&datasource.plugin),
                        datasource.tenant.to_string(),
                    ],
                )?;
            }
        }
        CliCommand::ListPlugins => {
            for plugin in db::plugin_settings::list(&conn)? {
                write_row(
                    output,
                    &[
                        field(&plugin.plugin_name),
                        plugin.enabled.to_string(),
                        plugin.updated_at.to_string(),
                    ],
                )?;
            }
        }
        CliCommand::DescribeApplication(id) => {
            let application = db::applications::get(&conn, id)?
                .ok_or_else(|| format!("application {id} not found"))?;
            let snapshot_count = db::applications::list_disable_snapshots(&conn, id)?.len();
            write_pair(output, "id", &application.id.to_string())?;
            write_pair(output, "name", &field(&application.name))?;
            write_pair(output, "enabled", &application.enabled.to_string())?;
            write_pair(output, "tenant", &application.tenant.to_string())?;
            write_pair(output, "disable_snapshots", &snapshot_count.to_string())?;
        }
        CliCommand::DescribeConnection => {
            if let Some(profile) = db::clusters::get_active(&conn)? {
                write_pair(output, "addr", &field(&profile.addr))?;
                write_pair(output, "tenant", &profile.tenant.to_string())?;
            } else {
                let addr = db::settings::get(&conn, "addr")?.unwrap_or_else(|| {
                    env::var("BEE_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:9999".to_string())
                });
                let tenant = db::settings::get(&conn, "tenant")?
                    .and_then(|value| value.parse::<u16>().ok())
                    .unwrap_or(0);
                write_pair(output, "addr", &field(&addr))?;
                write_pair(output, "tenant", &tenant.to_string())?;
            }
        }
        CliCommand::MigrateStatus | CliCommand::Reset | CliCommand::Help => unreachable!(),
    }
    Ok(())
}

fn reset_database(database_path: &Path, output: &mut dyn Write) -> Result<(), String> {
    remove_file_if_present(database_path)?;
    remove_file_if_present(&sidecar_path(database_path, "-wal"))?;
    remove_file_if_present(&sidecar_path(database_path, "-shm"))?;
    write_row(
        output,
        &[
            "reset".to_string(),
            field(&database_path.display().to_string()),
        ],
    )
}

fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove {}: {error}", path.display())),
    }
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar = path.as_os_str().to_os_string();
    sidecar.push(suffix);
    PathBuf::from(sidecar)
}

fn field(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

fn write_pair(output: &mut dyn Write, key: &str, value: &str) -> Result<(), String> {
    write_row(output, &[key.to_string(), value.to_string()])
}

fn write_row(output: &mut dyn Write, fields: &[String]) -> Result<(), String> {
    writeln!(output, "{}", fields.join("\t")).map_err(|error| format!("write output: {error}"))
}

fn write_text(output: &mut dyn Write, text: &str) -> Result<(), String> {
    writeln!(output, "{text}").map_err(|error| format!("write output: {error}"))
}
