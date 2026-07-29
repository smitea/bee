use serde::Serialize;

use crate::plugin_registry::{self, PluginRegistry, PluginSummary};

#[derive(Debug, Serialize, Clone)]
pub struct PluginSchema {
    pub name: String,
    pub adapters: serde_json::Value,
}

fn registry_static() -> &'static PluginRegistry {
    static REG: std::sync::OnceLock<PluginRegistry> = std::sync::OnceLock::new();
    REG.get_or_init(PluginRegistry::new)
}

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

#[tauri::command]
pub fn plugin_list() -> Vec<PluginSummary> {
    list_summaries()
}

#[tauri::command]
pub fn plugin_schema(plugin: String) -> PluginSchema {
    schema(&plugin)
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
}
