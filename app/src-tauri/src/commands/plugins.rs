use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub adapter: String,
    pub kind: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct PluginFieldSchema {
    pub name: String,
    pub kind: String,
    pub required: bool,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PluginSchema {
    pub name: String,
    pub fields: Vec<PluginFieldSchema>,
}

pub fn list() -> Vec<PluginInfo> {
    Vec::new()
}

pub fn schema(plugin: &str) -> PluginSchema {
    PluginSchema {
        name: plugin.to_string(),
        fields: vec![
            PluginFieldSchema {
                name: "url".into(),
                kind: "string".into(),
                required: true,
                description: Some("endpoint URL".into()),
            },
            PluginFieldSchema {
                name: "api_key".into(),
                kind: "string".into(),
                required: false,
                description: Some("API key (optional)".into()),
            },
        ],
    }
}

#[tauri::command]
pub fn plugin_list() -> Vec<PluginInfo> {
    list()
}

#[tauri::command]
pub fn plugin_schema(plugin: String) -> PluginSchema {
    schema(&plugin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_starts_empty() {
        assert!(list().is_empty());
    }

    #[test]
    fn schema_for_any_plugin_returns_static_fields() {
        let s = schema("binance_subscribe");
        assert_eq!(s.name, "binance_subscribe");
        assert!(!s.fields.is_empty());
        let names: Vec<&str> = s.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"url"));
    }

    #[test]
    fn schema_for_unknown_plugin_still_returns_static_fields() {
        let s = schema("unknown");
        assert_eq!(s.name, "unknown");
        assert!(!s.fields.is_empty());
    }
}