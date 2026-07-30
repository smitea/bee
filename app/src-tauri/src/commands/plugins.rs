use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};
use crate::plugin_registry::{self, PluginRegistry, PluginSummary};

#[derive(Debug, Serialize, Clone)]
pub struct PluginSchema {
    pub name: String,
    pub adapters: serde_json::Value,
}

#[derive(Debug, Serialize, Clone)]
pub struct DatasourceFormSchema {
    pub plugin_name: String,
    pub adapter: Option<String>,
    pub fields: Vec<DatasourceFormField>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DatasourceFormField {
    pub name: String,
    pub schema: serde_json::Value,
    pub required: bool,
}

fn registry_static() -> &'static PluginRegistry {
    static REG: std::sync::OnceLock<PluginRegistry> = std::sync::OnceLock::new();
    REG.get_or_init(PluginRegistry::new)
}

static INITIAL_SCAN_DONE: AtomicBool = AtomicBool::new(false);

pub fn list_summaries() -> Vec<PluginSummary> {
    registry_static().list_summaries()
}

pub fn schema(name: &str) -> PluginSchema {
    let manifest = registry_static().manifest(name);
    let adapters = match manifest {
        Some(m) => plugin_registry::schema_for(&m),
        None => plugin_registry::placeholder_schema(name),
    };
    PluginSchema {
        name: name.to_string(),
        adapters,
    }
}

pub fn default_plugin_dir() -> PathBuf {
    if let Ok(p) = std::env::var("BEE_PLUGIN_DIR") {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".bee").join("plugins");
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".bee").join("plugins");
        }
    }
    PathBuf::from(".bee").join("plugins")
}

pub fn ensure_initial_scan() {
    if INITIAL_SCAN_DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    let dir = default_plugin_dir();
    let _ = registry_static().scan_directory(&dir);
}

pub fn scan_directory_path(path: &Path) -> Vec<PluginSummary> {
    registry_static().scan_directory(path)
}

pub fn mark_initial_scan_done() {
    INITIAL_SCAN_DONE.store(true, Ordering::SeqCst);
}

fn first_adapter_name(adapters: &serde_json::Value) -> Option<String> {
    adapters
        .as_object()
        .and_then(|m| m.keys().next().map(|k| k.to_string()))
}

fn flatten_connection_fields(adapter: &serde_json::Value) -> Vec<DatasourceFormField> {
    let connection = adapter.get("connection");
    let Some(connection) = connection else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let props_obj = if let Some(p) = connection.get("properties").and_then(|v| v.as_object()) {
        p.clone()
    } else if let Some(p) = connection.as_object() {
        p.clone()
    } else {
        serde_json::Map::new()
    };
    for (name, schema) in props_obj.iter() {
        if name == "type" || name == "required" || name == "properties" {
            continue;
        }
        let required = schema
            .get("required")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        out.push(DatasourceFormField {
            name: name.clone(),
            schema: schema.clone(),
            required,
        });
    }
    out
}

#[tauri::command]
pub fn plugin_list() -> Vec<PluginSummary> {
    ensure_initial_scan();
    list_summaries()
}

#[tauri::command]
pub fn plugin_schema(plugin: String) -> PluginSchema {
    schema(&plugin)
}

#[tauri::command]
pub fn plugin_scan_directory(path: String) -> Vec<PluginSummary> {
    mark_initial_scan_done();
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    scan_directory_path(Path::new(trimmed))
}

#[tauri::command]
pub fn plugin_default_dir() -> String {
    default_plugin_dir().to_string_lossy().into_owned()
}

#[tauri::command]
pub fn plugin_last_dir(app: AppHandle) -> CmdResult<Option<String>> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    let conn = state.lock().map_err(CmdError::from)?;
    db::settings::get(&conn, "plugin_dir").map_err(CmdError::from)
}

#[tauri::command]
pub fn datasource_form_schema(plugin: String) -> DatasourceFormSchema {
    let plugin_str = plugin;
    let manifest = registry_static().manifest(&plugin_str);
    let adapters = match manifest.as_ref() {
        Some(m) => plugin_registry::schema_for(m),
        None => plugin_registry::placeholder_schema(&plugin_str),
    };
    let adapter = first_adapter_name(&adapters);
    let fields = adapter
        .as_ref()
        .and_then(|a| adapters.get(a))
        .map(flatten_connection_fields)
        .unwrap_or_default();
    DatasourceFormSchema {
        plugin_name: plugin_str,
        adapter,
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_registry() -> PluginRegistry {
        let reg = PluginRegistry::new();
        reg.insert_manifest(
            "id-a".into(),
            bee_plugin_sdk::PluginManifest {
                name: bee_plugin_sdk::PluginName("binance".into()),
                feature_version: "1.4.2".into(),
                abi_version: "v1".into(),
                adapters: vec![bee_plugin_sdk::AdapterDescriptor {
                    name: "subscribe".into(),
                    is_input: true,
                }],
                handlers: vec![bee_plugin_sdk::HandlerDescriptor {
                    name: "fib".into(),
                }],
            },
        );
        reg
    }

    #[test]
    fn empty_registry_returns_empty_plugin_list() {
        let reg = PluginRegistry::new();
        let summaries = reg.list_summaries();
        assert!(summaries.is_empty());
    }

    #[test]
    fn populated_registry_plugin_list_includes_all_summaries() {
        let reg = sample_registry();
        let summaries = reg.list_summaries();
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.id, "id-a");
        assert_eq!(s.name, "binance");
        assert_eq!(s.version, "1.4.2");
        assert_eq!(s.adapters, vec!["subscribe".to_string()]);
        assert_eq!(s.handlers, vec!["fib".to_string()]);
    }

    #[test]
    fn schema_for_known_plugin_returns_connection_shape() {
        let reg = sample_registry();
        let s = PluginSchema {
            name: "id-a".into(),
            adapters: plugin_registry::schema_for(
                &reg.manifest("id-a").expect("manifest"),
            ),
        };
        let adapter = s.adapters.get("subscribe").expect("adapter");
        let connection = adapter.get("connection").expect("connection");
        assert!(connection.get("url").is_some());
        assert!(connection.get("credentials").is_some());
        assert!(connection.get("rate_limit").is_some());
    }

    #[test]
    fn schema_for_unknown_plugin_returns_placeholder_with_requested_name() {
        let s = schema("does_not_exist");
        assert_eq!(s.name, "does_not_exist");
        let adapter = s.adapters.get("does_not_exist").expect("adapter");
        let connection = adapter.get("connection").expect("connection");
        assert!(connection.get("url").is_some());
        assert!(connection.get("credentials").is_some());
        assert!(connection.get("rate_limit").is_some());
    }

    #[test]
    fn datasource_form_schema_for_known_plugin_flattens_connection_fields() {
        let reg = sample_registry();
        let adapters = plugin_registry::schema_for(
            &reg.manifest("id-a").expect("manifest"),
        );
        let adapter = adapters.get("subscribe").expect("adapter");
        let fields = flatten_connection_fields(adapter);
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"url"));
        assert!(names.contains(&"credentials"));
        assert!(names.contains(&"rate_limit"));
        assert!(fields.iter().find(|f| f.name == "url").unwrap().required);
        assert!(!fields.iter().find(|f| f.name == "rate_limit").unwrap().required);
    }

    #[test]
    fn datasource_form_schema_for_unknown_plugin_uses_placeholder() {
        let s = datasource_form_schema("does_not_exist".to_string());
        assert_eq!(s.plugin_name, "does_not_exist");
        assert_eq!(s.adapter.as_deref(), Some("does_not_exist"));
        assert!(!s.fields.is_empty());
        assert!(s.fields.iter().any(|f| f.name == "url" && f.required));
    }

    #[test]
    fn first_adapter_name_returns_first_key() {
        let mut m = serde_json::Map::new();
        m.insert("alpha".into(), serde_json::json!({}));
        m.insert("beta".into(), serde_json::json!({}));
        let v = serde_json::Value::Object(m);
        assert_eq!(first_adapter_name(&v).as_deref(), Some("alpha"));
    }

    #[test]
    fn summaries_sorted_by_name_ascending() {
        let reg = PluginRegistry::new();
        let descriptors = vec![
            ("zeta-plugin", "z-id"),
            ("alpha-plugin", "a-id"),
            ("mike-plugin", "m-id"),
        ];
        for (name, id) in descriptors {
            reg.insert_manifest(
                id.into(),
                bee_plugin_sdk::PluginManifest {
                    name: bee_plugin_sdk::PluginName(name.into()),
                    feature_version: "0.0.1".into(),
                    abi_version: "v1".into(),
                    adapters: vec![],
                    handlers: vec![],
                },
            );
        }
        let summaries = reg.list_summaries();
        let names: Vec<&str> = summaries.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha-plugin", "mike-plugin", "zeta-plugin"]);
    }

    #[test]
    fn scan_directory_command_returns_empty_for_missing_path() {
        mark_initial_scan_done();
        let out = plugin_scan_directory("/tmp/bee_plugin_definitely_missing_xyz_9999".into());
        assert!(out.is_empty());
    }

    #[test]
    fn scan_directory_command_trims_whitespace_and_returns_empty_for_blank() {
        mark_initial_scan_done();
        let out = plugin_scan_directory("   ".into());
        assert!(out.is_empty());
    }

    #[test]
    fn scan_directory_command_returns_empty_when_path_is_nonexistent() {
        mark_initial_scan_done();
        let dir = std::env::temp_dir().join("bee_plugin_scan_nonexistent_xyz_unique");
        let _ = std::fs::remove_dir_all(&dir);
        let out = plugin_scan_directory(dir.to_string_lossy().into_owned());
        assert!(out.is_empty());
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn default_plugin_dir_respects_bee_plugin_dir_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("BEE_PLUGIN_DIR").ok();
        std::env::set_var("BEE_PLUGIN_DIR", "/custom/bee/plugin/dir");
        let dir = default_plugin_dir();
        assert_eq!(dir, PathBuf::from("/custom/bee/plugin/dir"));
        match prev {
            Some(v) => std::env::set_var("BEE_PLUGIN_DIR", v),
            None => std::env::remove_var("BEE_PLUGIN_DIR"),
        }
    }

    #[test]
    fn default_plugin_dir_falls_back_to_home_when_env_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev_plugin = std::env::var("BEE_PLUGIN_DIR").ok();
        let prev_home = std::env::var("HOME").ok();
        let prev_userprofile = std::env::var("USERPROFILE").ok();
        std::env::remove_var("BEE_PLUGIN_DIR");
        std::env::set_var("HOME", "/test/home");
        std::env::remove_var("USERPROFILE");
        let dir = default_plugin_dir();
        assert_eq!(dir, PathBuf::from("/test/home/.bee/plugins"));
        match prev_plugin {
            Some(v) => std::env::set_var("BEE_PLUGIN_DIR", v),
            None => std::env::remove_var("BEE_PLUGIN_DIR"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_userprofile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
}