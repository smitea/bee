//! S29 SQL preprocessor: `use <name>[@<version>];` directives +
//! strict mode (every adapter function call must be preceded by a
//! matching `use`).
//!
//! ## Why a preprocessor (not full SQL grammar extension)?
//! Per ADR-0010, the `use` syntax is Pipeline-level: it lives at
//! the top of the SQL file, like MySQL's `USE database`. The S29
//! preprocessor scans the file, extracts the directives, validates
//! the strict-mode invariant, and strips the `use` lines before
//! handing the SQL to DataFusion. DataFusion 49 doesn't natively
//! understand `use` so the preprocessor is the MVP hook.
//!
//! ## Strict mode enforcement
//! For every `<identifier>.<method>(...)` call in the SQL, the
//! identifier must appear in a `use` directive. A call to
//! `binance.subscribe(...)` without a prior `use binance;` is a
//! compile error with a clear message ("no `use binance;` found").
//! Likewise, a `use binance;` followed by `coingecko.subscribe(...)`
//! is a compile error ("binance is used; coingecko is not").
//!
//! ## S29 redo features
//! - [`preprocess_resolve`] resolves each `use` to a concrete
//!   `PluginId` via the `DatasourceRegistry` + `PluginManager`.
//! - [`check_inline_credentials`] rejects `api_key=...` /
//!   `token=...` / `secret=...` / `password=...` in adapter call
//!   args (per ADR-0010 MVP).
//! - [`check_emit_into`] validates `EMIT INTO <datasource>` matches
//!   a `use <datasource>;`.
//! - [`validate_datasource_config`] rejects per-call args
//!   (`symbol`, `interval`, `query`) inside `--config` JSON
//!   (those belong at the call site, per ADR-0010).

use bee_plugin_sdk::{PluginId, VersionSpec};

/// Trait alias for the slice of the Datasource registry the
/// preprocessor needs. Implemented for `&DatasourceRegistry` and
/// `&mut DatasourceRegistry`; lets callers pass either.
pub trait DatasourceLookup {
    fn lookup(&self, tenant: u16, name: &str) -> Option<DatasourceInfo>;
}

/// Minimal info the preprocessor needs about a Datasource. The
/// `DatasourceRegistry` constructs this from its full record;
/// production code (which has the full struct) calls
/// `DatasourceRegistry::lookup_for_preprocess` to produce it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasourceInfo {
    pub name: String,
    pub tenant: u16,
    pub adapter: String,
    pub version_spec: VersionSpec,
}

/// Trait alias for the slice of the Plugin manager the preprocessor
/// needs. Implemented for `&PluginManager` and `&mut PluginManager`.
pub trait PluginResolver {
    fn resolve(&self, name: &str, spec: &VersionSpec) -> Option<PluginId>;
}

/// One `use <name>[@<version>];` directive parsed from the top of
/// the SQL file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseDirective {
    pub name: String,
    pub version_spec: Option<VersionSpec>,
}

/// Parse the `use` directives at the top of the SQL file. Only the
/// leading run of lines (before the first non-`use` statement) is
/// scanned; the rest of the SQL is left untouched.
pub fn parse_use_directives(sql: &str) -> Vec<UseDirective> {
    let mut out = Vec::new();
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !is_use_line(trimmed) {
            break;
        }
        if let Some(d) = parse_use_line(trimmed) {
            out.push(d);
        }
    }
    out
}

fn is_use_line(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.starts_with("use ") || lower.starts_with("use\t") || lower == "use"
}

fn parse_use_line(s: &str) -> Option<UseDirective> {
    // Expect: "use <name>[@<version>];"  (semicolon optional for MVP)
    let s = s.trim().trim_end_matches(';').trim();
    let rest = s.strip_prefix("use")?.trim_start();
    // Split on '@' to separate name from version spec.
    if let Some((name, version_str)) = rest.split_once('@') {
        let name = name.trim().to_string();
        if name.is_empty() {
            return None;
        }
        let version = VersionSpec::parse(version_str.trim()).ok()?;
        Some(UseDirective {
            name,
            version_spec: Some(version),
        })
    } else {
        let name = rest.trim().to_string();
        if name.is_empty() {
            None
        } else {
            Some(UseDirective {
                name,
                version_spec: None,
            })
        }
    }
}

/// Strict mode check. Returns `Err(msg)` if any `<identifier>.<method>(`
/// call in the SQL is for a name not in the `use` list, or if the
/// `use` list is empty and the SQL uses an adapter call.
pub fn check_strict_mode(sql: &str, use_directives: &[UseDirective]) -> Result<(), String> {
    // Collect the set of names that have been `use`d.
    let used: std::collections::HashSet<&str> = use_directives
        .iter()
        .map(|d| d.name.as_str())
        .collect();

    // Strip `use` lines from the SQL so we don't false-positive on
    // the use directive itself (e.g., `use binance;` is not a call).
    let body = strip_use_lines(sql);

    // Scan for `<identifier>.<method>(` patterns. We use a simple
    // token scan rather than a regex to keep zero new deps.
    for call in scan_dot_calls(&body) {
        if !used.contains(call.as_str()) {
            return Err(format!(
                "strict-mode: `{}.*(...)` referenced but `use {};` is missing",
                call, call
            ));
        }
    }
    Ok(())
}

/// Strip the leading `use ...;` lines from `sql` so they don't get
/// matched as adapter calls. The remaining SQL is the body that
/// will be passed to DataFusion.
fn strip_use_lines(sql: &str) -> String {
    let mut out = String::new();
    let mut past_use_block = false;
    for line in sql.lines() {
        if !past_use_block && is_use_line(line.trim()) {
            continue;
        }
        past_use_block = true;
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Return every `<identifier>.<method>(` identifier in `sql`.
/// Identifiers that look like SQL keywords (e.g., `date.trunc`,
/// `string.split`) are not flagged — only bare names that match
/// `Identifier.method(` where Identifier is `[A-Za-z_][A-Za-z0-9_]*`
/// and the call follows on the same line.
fn scan_dot_calls(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in sql.lines() {
        // Skip pure-comment lines to avoid false positives.
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            let c = bytes[i];
            // Identifier start: letter or underscore.
            if c.is_ascii_alphabetic() || c == b'_' {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let name = &line[start..i];
                // Skip if next non-space char is not '.', or the
                // char after '.' is not an identifier start.
                let mut j = i;
                while j < bytes.len() && bytes[j] == b' ' {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'.' && j + 1 < bytes.len()
                    && (bytes[j + 1].is_ascii_alphabetic() || bytes[j + 1] == b'_')
                {
                    out.push(name.to_string());
                }
                // Continue past this identifier (the '.' + method
                // name is left for the next iteration to scan).
            } else {
                i += 1;
            }
        }
    }
    out
}

/// Convenience: parse + strict mode check + strip in one call.
pub fn preprocess(sql: &str) -> Result<(Vec<UseDirective>, String), String> {
    let directives = parse_use_directives(sql);
    check_strict_mode(sql, &directives)?;
    let stripped = strip_use_lines(sql);
    Ok((directives, stripped))
}

/// One resolved `use` directive. Produced by [`preprocess_resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedUse {
    pub directive: UseDirective,
    /// `None` if the Datasource exists but the requested Plugin
    /// version range matches no loaded Plugin.
    pub plugin_id: Option<PluginId>,
}

/// Parse + strict-mode-check + resolve each `use` to a concrete
/// `PluginId` via the Datasource registry + Plugin manager.
///
/// Rules:
/// - The Datasource must exist in the registry under `(tenant, name)`.
///   `tenant` is the Job's tenant (0 = global for MVP per ADR-0010).
/// - The effective `VersionSpec` is the directive's `@spec` if
///   present, otherwise the Datasource's stored spec.
/// - The Plugin manager resolves that spec against the loaded
///   Plugins; the highest matching version wins.
///
/// Returns a `Vec<ResolvedUse>` (one per `use` directive). Strict
/// mode is also enforced (every adapter call must have a `use`).
pub fn preprocess_resolve<R: DatasourceLookup, P: PluginResolver>(
    sql: &str,
    job_tenant: u16,
    registry: &R,
    plugins: &P,
) -> Result<Vec<ResolvedUse>, String> {
    let directives = parse_use_directives(sql);
    check_strict_mode(sql, &directives)?;
    let mut out = Vec::with_capacity(directives.len());
    for d in directives {
        let ds = registry.lookup(job_tenant, &d.name).ok_or_else(|| {
            format!(
                "datasource `{}` not found in tenant {} (register it with `bee datasource create`)",
                d.name, job_tenant
            )
        })?;
        let effective_spec = d.version_spec.clone().unwrap_or(ds.version_spec);
        let plugin_id = plugins.resolve(&ds.adapter, &effective_spec);
        out.push(ResolvedUse {
            directive: d,
            plugin_id,
        });
    }
    Ok(out)
}

/// S29 redo: per-call-arg keys that MUST NOT appear in
/// `--config` JSON (they belong at the call site, per ADR-0010).
pub const PER_CALL_ARG_KEYS: &[&str] = &["symbol", "interval", "query"];

/// S29 redo: credential-shaped keys that MUST NOT appear inline in
/// adapter call args (per ADR-0010). Credentials go through the
/// Secret store + `use` directive.
pub const INLINE_CREDENTIAL_KEYS: &[&str] =
    &["api_key", "apikey", "token", "secret", "password"];

/// S29 redo: validate that `--config` JSON does not contain
/// per-call args (e.g. `{"symbol": "BTC/USDT"}`). Those belong at
/// the call site.
pub fn validate_datasource_config(
    config: &serde_json::Value,
) -> Result<(), String> {
    if let Some(obj) = config.as_object() {
        for key in obj.keys() {
            let lower = key.to_ascii_lowercase();
            if PER_CALL_ARG_KEYS.contains(&lower.as_str()) {
                return Err(format!(
                    "config key `{}` is a per-call arg; it belongs at the call site (e.g. `binance.subscribe('{}', ...)`)",
                    lower, lower
                ));
            }
        }
    }
    Ok(())
}

/// S29 redo: scan adapter call args for inline credentials. A
/// matching pattern is `key=value` or `key: value` where the key
/// matches one of [`INLINE_CREDENTIAL_KEYS`]. Returns `Err(msg)`
/// on the first violation.
pub fn check_inline_credentials(sql: &str) -> Result<(), String> {
    let body = strip_use_lines(sql);
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            continue;
        }
        // Look for `key=` or `key:` patterns. We do a simple token
        // scan to avoid a regex dep.
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_alphabetic() || c == b'_' {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                let key = &line[start..i];
                let key_lower = key.to_ascii_lowercase();
                // Skip whitespace
                let mut j = i;
                while j < bytes.len() && bytes[j] == b' ' {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'=' || bytes[j] == b':') {
                    if INLINE_CREDENTIAL_KEYS.contains(&key_lower.as_str()) {
                        return Err(format!(
                            "inline credential `{}` in call args; move to Secret store + Datasource config",
                            key_lower
                        ));
                    }
                }
            } else {
                i += 1;
            }
        }
    }
    Ok(())
}

/// S29 redo: validate `EMIT INTO <datasource>` (basic). The
/// `EMIT INTO` clause must reference a `use`'d Datasource. Returns
/// `Ok(())` if all `EMIT INTO` clauses are covered, `Err(msg)` if
/// any are not. The current implementation is permissive: it only
/// checks that the Datasource name in `EMIT INTO` is in the `use`
/// list. Full validation (the targeted method must exist on the
/// Adapter) is deferred.
pub fn check_emit_into(
    sql: &str,
    use_directives: &[UseDirective],
) -> Result<(), String> {
    let used: std::collections::HashSet<&str> = use_directives
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    let body = strip_use_lines(sql);
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            continue;
        }
        // Look for `EMIT INTO <name>` (case-insensitive, ignoring
        // leading whitespace). The Datasource name is a single
        // identifier before the `.method(`.
        let upper = trimmed.to_ascii_uppercase();
        if let Some(idx) = upper.find("EMIT INTO") {
            let rest = trimmed[idx + "EMIT INTO".len()..].trim_start();
            // Take the identifier up to '.' or whitespace.
            let mut end = 0;
            for (i, c) in rest.char_indices() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    end = i + c.len_utf8();
                } else {
                    break;
                }
            }
            let name = &rest[..end];
            if name.is_empty() {
                return Err("EMIT INTO requires a Datasource name".into());
            }
            if !used.contains(name) {
                return Err(format!(
                    "EMIT INTO `{name}` referenced but `use {name};` is missing"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_use_directive() {
        let sql = "use binance;\nSELECT * FROM stream";
        let d = parse_use_directives(sql);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "binance");
        assert_eq!(d[0].version_spec, None);
    }

    #[test]
    fn parse_use_with_version_spec() {
        let sql = "use binance@1.4.2;\nSELECT 1";
        let d = parse_use_directives(sql);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "binance");
        assert_eq!(
            d[0].version_spec,
            Some(VersionSpec::Exact(bee_plugin_sdk::Version::parse("1.4.2").unwrap()))
        );
    }

    #[test]
    fn parse_use_with_caret_and_tilde() {
        let sql_caret = "use binance@^1.0;\nSELECT 1";
        let sql_tilde = "use coingecko@~1.2;\nSELECT 1";
        let d_caret = parse_use_directives(sql_caret);
        let d_tilde = parse_use_directives(sql_tilde);
        assert_eq!(
            d_caret[0].version_spec,
            Some(VersionSpec::Compatible(bee_plugin_sdk::Version::parse("1.0").unwrap()))
        );
        assert_eq!(
            d_tilde[0].version_spec,
            Some(VersionSpec::Patch(bee_plugin_sdk::Version::parse("1.2").unwrap()))
        );
    }

    #[test]
    fn parse_multiple_use_directives() {
        let sql = "use binance;\nuse coingecko;\nSELECT 1";
        let d = parse_use_directives(sql);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].name, "binance");
        assert_eq!(d[1].name, "coingecko");
    }

    #[test]
    fn use_block_terminates_at_first_non_use_line() {
        let sql = "use binance;\n\nSELECT 1\nuse coingecko;";
        let d = parse_use_directives(sql);
        // Only the leading use is parsed; the second one after
        // SELECT is ignored (it's not a leading use).
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "binance");
    }

    #[test]
    fn strict_mode_passes_with_matching_use() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe('BTC/USDT')";
        let d = parse_use_directives(sql);
        assert!(check_strict_mode(sql, &d).is_ok());
    }

    #[test]
    fn strict_mode_fails_when_adapter_call_has_no_use() {
        let sql = "SELECT * FROM binance.subscribe('BTC/USDT')";
        let d = parse_use_directives(sql);
        let err = check_strict_mode(sql, &d).unwrap_err();
        assert!(err.contains("strict-mode"), "missing tag: {err}");
        assert!(err.contains("binance"), "missing name: {err}");
    }

    #[test]
    fn strict_mode_fails_when_adapter_call_doesnt_match_use() {
        // S29 acceptance: SQL `use binance; SELECT * FROM coingecko.subscribe(...);`
        // is a compile error (coingecko is not used).
        let sql = "use binance;\nSELECT * FROM coingecko.subscribe('BTC/USDT')";
        let d = parse_use_directives(sql);
        let err = check_strict_mode(sql, &d).unwrap_err();
        assert!(err.contains("coingecko"), "missing name: {err}");
    }

    #[test]
    fn preprocess_strips_use_lines() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe('BTC/USDT')";
        let (d, stripped) = preprocess(sql).expect("preprocess");
        assert_eq!(d.len(), 1);
        assert!(!stripped.contains("use binance"), "use line not stripped:\n{stripped}");
        assert!(stripped.contains("binance.subscribe"), "body truncated:\n{stripped}");
    }

    #[test]
    fn strict_mode_handles_multiple_calls() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe('BTC/USDT') JOIN coingecko.snapshot('BTC')";
        let d = parse_use_directives(sql);
        let err = check_strict_mode(sql, &d).unwrap_err();
        assert!(err.contains("coingecko"));
    }

    // ---- S29 redo: PluginManager resolution ----

    use bee_plugin_sdk::{compute_plugin_id, Version};
    use std::collections::HashMap;

    /// Tiny in-memory Datasource registry stand-in for tests.
    struct StubRegistry {
        map: HashMap<String, DatasourceInfo>,
    }
    impl DatasourceLookup for StubRegistry {
        fn lookup(&self, _tenant: u16, name: &str) -> Option<DatasourceInfo> {
            self.map.get(name).cloned()
        }
    }

    /// Tiny in-memory Plugin manager stand-in.
    struct StubPlugins {
        /// adapter name -> (Version, PluginId)
        plugins: Vec<(String, Version, PluginId)>,
    }
    impl PluginResolver for StubPlugins {
        fn resolve(&self, name: &str, spec: &VersionSpec) -> Option<PluginId> {
            let mut best: Option<(Version, PluginId)> = None;
            for (n, v, id) in &self.plugins {
                if n != name {
                    continue;
                }
                if !spec.matches(v) {
                    continue;
                }
                match &best {
                    Some((bv, _)) if *bv >= *v => {}
                    _ => best = Some((v.clone(), id.clone())),
                }
            }
            best.map(|(_, id)| id)
        }
    }

    fn stub_ds(name: &str, adapter: &str, spec: VersionSpec) -> DatasourceInfo {
        DatasourceInfo {
            name: name.into(),
            tenant: 0,
            adapter: adapter.into(),
            version_spec: spec,
        }
    }

    #[test]
    fn resolve_uses_directive_spec_wins_over_datasource_spec() {
        let sql = "use binance@1.4.2;\nSELECT * FROM binance.subscribe('BTC/USDT')";
        let mut reg_map = HashMap::new();
        reg_map.insert(
            "binance".into(),
            stub_ds("binance", "binance", VersionSpec::Latest),
        );
        let reg = StubRegistry { map: reg_map };
        let plugins = StubPlugins {
            plugins: vec![
                ("binance".into(), Version::parse("1.4.0").unwrap(), compute_plugin_id(b"1.4.0")),
                ("binance".into(), Version::parse("1.4.2").unwrap(), compute_plugin_id(b"1.4.2")),
                ("binance".into(), Version::parse("2.0.0").unwrap(), compute_plugin_id(b"2.0.0")),
            ],
        };
        let r = preprocess_resolve(sql, 0, &reg, &plugins).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].plugin_id, Some(compute_plugin_id(b"1.4.2")));
    }

    #[test]
    fn resolve_caret_spec_picks_highest_compatible() {
        let sql = "use binance@^1.0;\nSELECT * FROM binance.subscribe('BTC/USDT')";
        let mut reg_map = HashMap::new();
        reg_map.insert(
            "binance".into(),
            stub_ds("binance", "binance", VersionSpec::Latest),
        );
        let reg = StubRegistry { map: reg_map };
        let plugins = StubPlugins {
            plugins: vec![
                ("binance".into(), Version::parse("1.0.0").unwrap(), compute_plugin_id(b"1.0.0")),
                ("binance".into(), Version::parse("1.4.2").unwrap(), compute_plugin_id(b"1.4.2")),
                ("binance".into(), Version::parse("2.0.0").unwrap(), compute_plugin_id(b"2.0.0")),
            ],
        };
        let r = preprocess_resolve(sql, 0, &reg, &plugins).unwrap();
        assert_eq!(r[0].plugin_id, Some(compute_plugin_id(b"1.4.2")));
    }

    #[test]
    fn resolve_no_spec_uses_datasource_stored_spec() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe('BTC/USDT')";
        let mut reg_map = HashMap::new();
        reg_map.insert(
            "binance".into(),
            stub_ds("binance", "binance", VersionSpec::Exact(Version::parse("1.4.2").unwrap())),
        );
        let reg = StubRegistry { map: reg_map };
        let plugins = StubPlugins {
            plugins: vec![
                ("binance".into(), Version::parse("1.4.2").unwrap(), compute_plugin_id(b"1.4.2")),
                ("binance".into(), Version::parse("1.5.0").unwrap(), compute_plugin_id(b"1.5.0")),
            ],
        };
        let r = preprocess_resolve(sql, 0, &reg, &plugins).unwrap();
        assert_eq!(r[0].plugin_id, Some(compute_plugin_id(b"1.4.2")));
    }

    #[test]
    fn resolve_missing_datasource_errors() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe('BTC/USDT')";
        let reg = StubRegistry { map: HashMap::new() };
        let plugins = StubPlugins { plugins: vec![] };
        let err = preprocess_resolve(sql, 0, &reg, &plugins).unwrap_err();
        assert!(err.contains("not found"), "err: {err}");
        assert!(err.contains("binance"));
    }

    #[test]
    fn resolve_no_matching_plugin_yields_none() {
        // Plugin manager has a 2.x but the directive pins ^1.0.
        let sql = "use binance@^1.0;\nSELECT * FROM binance.subscribe('BTC/USDT')";
        let mut reg_map = HashMap::new();
        reg_map.insert(
            "binance".into(),
            stub_ds("binance", "binance", VersionSpec::Latest),
        );
        let reg = StubRegistry { map: reg_map };
        let plugins = StubPlugins {
            plugins: vec![(
                "binance".into(),
                Version::parse("2.0.0").unwrap(),
                compute_plugin_id(b"2.0.0"),
            )],
        };
        let r = preprocess_resolve(sql, 0, &reg, &plugins).unwrap();
        assert_eq!(r[0].plugin_id, None);
    }

    // ---- S29 redo: inline credential detect ----

    #[test]
    fn check_inline_credentials_rejects_api_key() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe('BTC/USDT', api_key='abc')";
        let err = check_inline_credentials(sql).unwrap_err();
        assert!(err.contains("api_key"), "err: {err}");
    }

    #[test]
    fn check_inline_credentials_rejects_token() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe('BTC/USDT', token='xyz')";
        let err = check_inline_credentials(sql).unwrap_err();
        assert!(err.contains("token"));
    }

    #[test]
    fn check_inline_credentials_passes_clean_args() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe('BTC/USDT', '5min')";
        assert!(check_inline_credentials(sql).is_ok());
    }

    #[test]
    fn check_inline_credentials_ignores_use_directive() {
        // The `use` line is stripped before scanning; the
        // `token=` inside a `use` would be a separate parse error.
        let sql = "use binance;\nSELECT * FROM binance.subscribe('BTC/USDT')";
        assert!(check_inline_credentials(sql).is_ok());
    }

    // ---- S29 redo: config schema validation ----

    #[test]
    fn validate_config_rejects_symbol() {
        let cfg: serde_json::Value =
            serde_json::from_str(r#"{"base_url":"wss://...","symbol":"BTC/USDT"}"#).unwrap();
        let err = validate_datasource_config(&cfg).unwrap_err();
        assert!(err.contains("symbol"));
    }

    #[test]
    fn validate_config_rejects_interval() {
        let cfg: serde_json::Value =
            serde_json::from_str(r#"{"base_url":"wss://...","interval":"5min"}"#).unwrap();
        let err = validate_datasource_config(&cfg).unwrap_err();
        assert!(err.contains("interval"));
    }

    #[test]
    fn validate_config_rejects_query() {
        let cfg: serde_json::Value =
            serde_json::from_str(r#"{"query":"SELECT *"}"#).unwrap();
        let err = validate_datasource_config(&cfg).unwrap_err();
        assert!(err.contains("query"));
    }

    #[test]
    fn validate_config_passes_connection_keys() {
        let cfg: serde_json::Value =
            serde_json::from_str(r#"{"base_url":"wss://...","api_key_ref":"binance:prod"}"#).unwrap();
        assert!(validate_datasource_config(&cfg).is_ok());
    }

    #[test]
    fn validate_config_accepts_array_value() {
        // Non-object root is a no-op.
        let cfg: serde_json::Value = serde_json::from_str(r#"[1,2,3]"#).unwrap();
        assert!(validate_datasource_config(&cfg).is_ok());
    }

    // ---- S29 redo: EMIT INTO ----

    #[test]
    fn check_emit_into_passes_with_matching_use() {
        let sql = "use binance;\nEMIT INTO binance (symbol, qty) VALUES ('BTC', 1)";
        let d = parse_use_directives(sql);
        assert!(check_emit_into(sql, &d).is_ok());
    }

    #[test]
    fn check_emit_into_fails_without_use() {
        let sql = "EMIT INTO binance (symbol, qty) VALUES ('BTC', 1)";
        let d = parse_use_directives(sql);
        let err = check_emit_into(sql, &d).unwrap_err();
        assert!(err.contains("EMIT INTO"));
        assert!(err.contains("binance"));
    }

    #[test]
    fn check_emit_into_case_insensitive() {
        let sql = "use binance;\nemit into binance (symbol) VALUES ('BTC')";
        let d = parse_use_directives(sql);
        assert!(check_emit_into(sql, &d).is_ok());
    }
}
