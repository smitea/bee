use serde::Serialize;

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
    list_summaries()
}

#[tauri::command]
pub fn plugin_schema(plugin: String) -> PluginSchema {
    schema(&plugin)
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
}
