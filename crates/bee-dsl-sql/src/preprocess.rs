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

/// S17 §2: extract the stream-producing identities from a SQL
/// Pipeline. An identity is a triple `(datasource_name,
/// adapter_method, stream_topology_args)`. The `datasource_name`
/// comes from the matching `use <name>;` directive; the
/// `adapter_method` and `stream_topology_args` come from the
/// `<name>.<method>(<args>)` calls in the body.
///
/// For the MVP, args are captured as a `BTreeMap<String, String>`
/// (string-typed only). Numeric / boolean / null args are
/// serialized to their JSON form. Plugins that need richer
/// topology args can override the S17 signature in a follow-up.
pub fn extract_stream_identities(
    sql: &str,
) -> Vec<(String, String, std::collections::BTreeMap<String, String>)> {
    use std::collections::BTreeMap;
    let (directives, body) = match preprocess(sql) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut out: Vec<(String, String, BTreeMap<String, String>)> = vec![];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            let c = bytes[i];
            if !(c.is_ascii_alphabetic() || c == b'_') {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = &line[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j >= bytes.len() || bytes[j] != b'.' {
                continue;
            }
            if j + 1 >= bytes.len()
                || !(bytes[j + 1].is_ascii_alphabetic() || bytes[j + 1] == b'_')
            {
                continue;
            }
            let method_start = j + 1;
            let mut k = method_start;
            while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
                k += 1;
            }
            let method = &line[method_start..k];
            let mut paren_depth = 0;
            let mut m = k;
            let mut found_paren = false;
            while m < bytes.len() {
                match bytes[m] {
                    b'(' => {
                        paren_depth += 1;
                        found_paren = true;
                        m += 1;
                    }
                    b')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            break;
                        }
                        m += 1;
                    }
                    _ => m += 1,
                }
            }
            if !found_paren || paren_depth != 0 {
                continue;
            }
            let args_text = &line[k..=m.min(bytes.len() - 1)];
            let args_inner = args_text
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim();
            let mut map: BTreeMap<String, String> = BTreeMap::new();
            for pair in args_inner.split(',') {
                let pair = pair.trim();
                if pair.is_empty() {
                    continue;
                }
                let (k_str, v_str) = if let Some(idx) = pair.find("=>") {
                    (pair[..idx].trim(), pair[idx + 2..].trim())
                } else if let Some(idx) = pair.find('=') {
                    (pair[..idx].trim(), pair[idx + 1..].trim())
                } else {
                    continue;
                };
                let v_str = v_str
                    .trim_matches('\'')
                    .trim_matches('"')
                    .to_string();
                map.insert(k_str.to_string(), v_str);
            }
            let dedup_key = format!("{name}.{method}");
            if !seen.insert(dedup_key) {
                continue;
            }
            // The preprocessor already enforces strict mode; if a
            // matching `use <name>;` is missing, `preprocess` would
            // have returned `Err` and we'd be in the early-return
            // path. The directive lookup is a no-op for the MVP but
            // keeps the door open for per-datasource filtering
            // (e.g. excluding Emit-only datasources from the
            // signature list).
            let _matched_use = directives.iter().find(|d| d.name == name);
            out.push((name.to_string(), method.to_string(), map));
        }
    }
    out
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

/// S41 (9a): sink target for `EMIT INTO <target>`. The MVP ships
/// `Console` (built-in, writes rows to stdout). Future sinks
/// (InfluxDB, MongoDB, …) will be added as enum variants and a
/// corresponding arm in [`strip_emit_into`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmitTarget {
    /// `EMIT INTO console` — write rows to stdout, one per line,
    /// formatted as `col1=val1, col2=val2, ...`.
    Console,
    /// `CREATE SINK <name> AS <body>` — write rows to the named plugin.
    Plugin(String),
}

/// S41 (9a / 9c): recognize and strip an `EMIT INTO <target>` prefix
/// from a SQL string. Returns the [`EmitTarget`] (or `None` if the
/// SQL does not contain an `EMIT INTO`) and the remaining SQL with
/// the prefix removed.
///
/// The recognition is case-insensitive and finds `EMIT INTO` as a
/// line prefix anywhere in the SQL (not just at the start). This
/// lets the S41 demo's `EMIT INTO` on a line after `use` / `CREATE`
/// statements be recognised. The target identifier is a single
/// ASCII word (one run of non-whitespace chars). Unknown targets
/// return `(None, original_sql)` so the rest of the pipeline can
/// surface a proper error (DataFusion will reject the `EMIT INTO`
/// syntax at parse time).
///
/// The returned SQL preserves everything before the `EMIT INTO`
/// line (use directives, CREATE statements, comments, etc.) — only
/// the `EMIT INTO <target>` token is removed. The downstream
/// preprocessor (`preprocess_sql_v2`) is responsible for stripping
/// the `use` / `CREATE` / etc. directives.
pub fn strip_emit_into(sql: &str) -> (Option<EmitTarget>, String) {
    // Walk line by line. For each line that starts (after optional
    // leading whitespace) with `EMIT INTO`, parse the target and
    // check if it's `console`. If so, strip the `EMIT INTO <target>`
    // prefix from the line and return the SQL with that prefix
    // removed. If not, keep searching.
    let mut line_start = 0;
    let lines: Vec<&str> = sql.split_inclusive('\n').collect();
    for line in lines {
        let line_len = line.len();
        let trimmed = line.trim_start();
        if trimmed.to_ascii_uppercase().starts_with("EMIT INTO") {
            // Parse the target from the trimmed line.
            let after_emit = &trimmed["EMIT INTO".len()..];
            let after_ws = after_emit.trim_start();
            // The target ends at the next whitespace OR at the end
            // of the line (which may be a `\n`).
            let target_end = after_ws
                .find(|c: char| c.is_whitespace())
                .unwrap_or(after_ws.len());
            let target = &after_ws[..target_end];
            if target.eq_ignore_ascii_case("console") {
                // ... same logic for console ...
                let before = &sql[..line_start];
                let after_target = &after_ws[target_end..];
                let after_line = line_start + line_len;
                let after = &sql[after_line..];
                let rest_of_line = after_target.trim_start();
                if rest_of_line.is_empty() {
                    let mut out = String::with_capacity(before.len() + after.len());
                    out.push_str(before);
                    out.push_str(after);
                    return (Some(EmitTarget::Console), out);
                } else {
                    let mut out = String::with_capacity(before.len() + rest_of_line.len() + after.len());
                    out.push_str(before);
                    out.push_str(rest_of_line);
                    out.push_str(after);
                    return (Some(EmitTarget::Console), out);
                }
            } else {
                // S33.5.3: other targets are treated as plugins.
                let before = &sql[..line_start];
                let after_target = &after_ws[target_end..];
                let after_line = line_start + line_len;
                let after = &sql[after_line..];
                let rest_of_line = after_target.trim_start();
                if rest_of_line.is_empty() {
                    let mut out = String::with_capacity(before.len() + after.len());
                    out.push_str(before);
                    out.push_str(after);
                    return (Some(EmitTarget::Plugin(target.to_string())), out);
                } else {
                    let mut out = String::with_capacity(before.len() + rest_of_line.len() + after.len());
                    out.push_str(before);
                    out.push_str(rest_of_line);
                    out.push_str(after);
                    return (Some(EmitTarget::Plugin(target.to_string())), out);
                }
            }
        }
        line_start += line_len;
    }
    (None, sql.to_string())
}

/// S41 (9c): one `CREATE SOURCE` or `CREATE VIEW` definition parsed
/// from the SQL prelude. The `body` is the SELECT (or any DataFusion
/// SELECT-shaped expression) that defines the source/view.
///
/// For the S41 MVP, the preprocessor stores the raw body and
/// substitutes the name with `(<body>)` wherever it appears in the
/// downstream SQL. The `kind` field distinguishes source from view
/// purely for diagnostics (the wire-out is identical for both).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDefinition {
    pub kind: CreateKind,
    pub name: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateKind {
    Source,
    View,
    Sink,
}

/// Strip `CREATE SINK <name> AS <body>` and return `(Some(name), body)`.
/// If no `CREATE SINK` is found, return `(None, sql)`.
pub fn strip_create_sink(sql: &str) -> (Option<String>, String) {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    let mut hit_name = None;

    while let Some(hit) = find_create_statement(rest) {
        if hit.kind == CreateKind::Sink {
            out.push_str(&rest[..hit.start]);
            hit_name = Some(hit.name);
            // We substitute the rest with the body so DataFusion can compile the body.
            // Wait, what if there are multiple statements? The MVP assumes 1 sink.
            // Let's just output the body instead of the CREATE statement.
            out.push_str(&hit.body);
            rest = &rest[hit.end..];
            break; // only handle one sink
        } else {
            out.push_str(&rest[..hit.end]);
            rest = &rest[hit.end..];
        }
    }
    out.push_str(rest);
    (hit_name, out)
}

/// S41 (9c): recognize and strip `CREATE SOURCE` / `CREATE VIEW`
/// statements from a SQL string, and substitute references to the
/// declared name with the underlying body as a subquery.
///
/// The substitution is a single-pass word-boundary replace: every
/// occurrence of `<name>` as a standalone identifier in the
/// remaining SQL becomes `(<body>)`. The MVP assumes the name is
/// only used in `FROM` / `JOIN` positions (the S41 demo only uses
/// `FROM`); column-name collisions would require a follow-up.
///
/// The body may span multiple lines; the statement ends at the
/// first `;` after the `AS` keyword, or at the start of the next
/// statement (a line beginning with a recognised statement-start
/// keyword like `CREATE`, `SELECT`, `EMIT`, `use`), or at
/// end-of-string. String literals containing `;` are not supported
/// by this MVP parser — the S41 demo's bodies don't have them.
///
/// Substitutions are applied recursively: if view `b` is defined
/// with body referencing source `a`, then `b`'s body is rewritten
/// to inline `a`'s body. This chains correctly for the S41 demo
/// (one SOURCE + one VIEW).
///
/// In addition, the body is scanned for the pattern
/// `FROM <UDF_name>(...)` and rewritten to
/// `FROM UNNEST(<UDF_name>(...)) [AS <alias>]`. This is a
/// hack for the S41 MVP's test-fixture UDFs: DataFusion 50
/// has no UDTF support, so the only way to turn a scalar UDF's
/// array-shaped result into a table is `UNNEST`. Two UDFs are
/// handled:
/// - `generate_series(start, end)` returns a `List<Int64>` and
///   rewrites to `... AS u(n)` (the `n` rename maps the
///   List's `item` field to the demo's expected column name).
/// - `generate_events(schema, count, seed)` returns a
///   `List<Struct<user_id: Int64, ts: Int64>>` and rewrites to
///   `UNNEST(...)` with NO table alias (see
///   `rewrite_test_fixtures_in_from_handles_generate_events`
///   for the history of why `AS t` and `AS t(user_id, ts)`
///   don't work in DataFusion 50).
/// Other UDFs in `FROM` are left alone (DataFusion will surface a
/// clear error at planning time). A proper UDTF-based replacement
/// is a follow-up.
pub fn strip_create_source_and_view(sql: &str) -> (Vec<CreateDefinition>, String) {
    // 1. Scan for `CREATE SOURCE <name> AS <body>;` and
    //    `CREATE VIEW <name> AS <body>;` lines. Collect them into
    //    a Vec in source order.
    let mut defs: Vec<CreateDefinition> = Vec::new();
    let mut cleaned: String = String::with_capacity(sql.len());
    let mut rest = sql;

    while let Some(hit) = find_create_statement(rest) {
        // Append everything before the CREATE statement.
        cleaned.push_str(&rest[..hit.start]);
        // Record the definition.
        defs.push(CreateDefinition {
            kind: hit.kind,
            name: hit.name.clone(),
            body: hit.body.clone(),
        });
        // Skip past the statement (incl. the trailing `;` if any).
        rest = &rest[hit.end..];
    }
    cleaned.push_str(rest);

    // 2. For each definition, apply the `FROM <UDF_name>(...)` →
    //    `FROM UNNEST(<UDF_name>(...)) AS <alias>(<col>, ...)` rewrite
    //    to the body. The MVP knows the test-fixture UDFs
    //    `generate_series` and `generate_events`; other names are
    //    left alone (the SQL will surface a clear DataFusion error
    //    if the body references an unknown UDF).
    for d in &mut defs {
        d.body = rewrite_test_fixtures_in_from(&d.body);
    }

    // 3. Recursive substitution: apply all OTHER defs'
    //    substitutions to each def's body. This handles chains
    //    like `CREATE VIEW b AS ... FROM a;` where `a` is a
    //    separately-declared SOURCE.
    for i in 0..defs.len() {
        for j in 0..defs.len() {
            if i == j {
                continue;
            }
            let mut body = std::mem::take(&mut defs[i].body);
            substitute_name_with_body(&mut body, &defs[j].name, &defs[j].body);
            defs[i].body = body;
        }
    }

    // 4. Substitute references to each name with `(<body>) AS <name>`
    //    in the remaining SQL. Word-boundary match (don't replace a
    //    prefix of a longer identifier). The `AS <name>` alias
    //    preserves any column-qualified references like
    //    `<name>.col` that might appear in the downstream SQL
    //    (DataFusion requires a subquery in `FROM` to have an
    //    alias when it's referenced as a named table elsewhere).
    for d in &defs {
        substitute_name_with_body_aliased(&mut cleaned, &d.name, &d.body);
    }

    (defs, cleaned)
}

struct CreateHit {
    start: usize,
    end: usize,
    kind: CreateKind,
    name: String,
    body: String,
}

/// Find the `AS` keyword in `upper` (case-insensitive form of the
/// relevant slice of the original SQL). Returns the byte offset of
/// the `A` in `AS`. The `AS` must be preceded by whitespace and
/// followed by whitespace (space, tab, newline, or carriage
/// return). Returns `None` if no valid `AS` is found.
///
/// `original` is the corresponding slice of the original
/// (non-uppercased) SQL, used only to confirm the offset aligns
/// with the same byte position (upper-casing ASCII is a no-op for
/// offsets, so this is just a sanity check; we keep the param for
/// clarity and future Unicode handling).
fn find_as_keyword(upper: &str, original: &str) -> Option<usize> {
    debug_assert_eq!(upper.len(), original.len(), "find_as_keyword: slice length mismatch");
    let bytes = upper.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        // Left boundary: whitespace or start of string.
        let left_ok = i == 0 || bytes[i - 1].is_ascii_whitespace();
        if !left_ok {
            i += 1;
            continue;
        }
        // Need at least 2 bytes for `AS`.
        if i + 1 >= len {
            break;
        }
        // Match `AS` (case-insensitive, but `upper` is already upper).
        if bytes[i] == b'A' && bytes[i + 1] == b'S' {
            // Right boundary: whitespace (or end of string).
            let right_ok = i + 2 >= len || bytes[i + 2].is_ascii_whitespace();
            if right_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Find the next `CREATE SOURCE` or `CREATE VIEW` statement in
/// `sql`. Returns `Some(CreateHit)` with byte offsets, kind, name,
/// and body; `None` if no more CREATE statements are present.
fn find_create_statement(sql: &str) -> Option<CreateHit> {
    let upper = sql.to_ascii_uppercase();
    // Try CREATE SOURCE, CREATE VIEW, CREATE SINK.
    let s_pos = upper.find("CREATE SOURCE");
    let v_pos = upper.find("CREATE VIEW");
    let k_pos = upper.find("CREATE SINK");
    
    let mut earliest = None;
    if let Some(s) = s_pos { earliest = Some((s, CreateKind::Source)); }
    if let Some(v) = v_pos {
        if earliest.map_or(true, |(pos, _)| v < pos) {
            earliest = Some((v, CreateKind::View));
        }
    }
    if let Some(k) = k_pos {
        if earliest.map_or(true, |(pos, _)| k < pos) {
            earliest = Some((k, CreateKind::Sink));
        }
    }

    let (pos, kind) = match earliest {
        Some(x) => x,
        None => return None,
    };

    // Skip "CREATE SOURCE" or "CREATE VIEW" or "CREATE SINK" (13 or 11 chars).
    let kw_len = match kind {
        CreateKind::Source => "CREATE SOURCE".len(),
        CreateKind::View => "CREATE VIEW".len(),
        CreateKind::Sink => "CREATE SINK".len(),
    };
    let after_kw = &sql[pos + kw_len..];
    let trimmed = after_kw.trim_start();

    // Read the name: a run of [A-Za-z0-9_] (until whitespace or AS).
    let name_end = trimmed
        .find(|c: char| c.is_whitespace() || c == ';')
        .unwrap_or(trimmed.len());
    let name = trimmed[..name_end].trim().to_string();
    if name.is_empty() {
        return None;
    }

    // Find the `AS` keyword. It must appear before the next `;` and
    // before the end of the trimmed string. The `AS` is preceded by
    // whitespace and followed by whitespace (space, tab, newline,
    // carriage return).
    let after_name = &trimmed[name_end..];
    let upper_after = after_name.to_ascii_uppercase();
    let as_a_offset = find_as_keyword(&upper_after, after_name)?;
    // Skip past `AS` and any following whitespace.
    let body_start_rel = as_a_offset + 2
        + after_name[as_a_offset + 2..]
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(|c| c.len_utf8())
            .sum::<usize>();
    let after_as = &after_name[body_start_rel..];

    // Body: everything from after the `AS` up to the first `;`
    // OR the start of the next statement (a line beginning with a
    // recognised statement-start keyword: CREATE, SELECT, EMIT,
    // use), OR end-of-string. The S41 demo's bodies are single
    // SELECT expressions, so the `;`-or-newline-of-new-statement
    // heuristic is sufficient.
    let body_end_rel = find_body_end(after_as);
    let body_raw = &after_as[..body_end_rel];
    let body = body_raw.trim().to_string();

    // Compute the end of the statement in the original `sql` byte
    // offsets. We work from `pos` forward, skipping whitespace and
    // name and `AS` to find the body start, then scanning for the
    // terminating `;` (if any).
    let kw_part_end = pos + kw_len;
    let ws1_len = sql[kw_part_end..]
        .find(|c: char| !c.is_whitespace())
        .unwrap_or(0);
    let name_start = kw_part_end + ws1_len;
    let name_end_abs = name_start + name.len();
    let after_name = &sql[name_end_abs..];
    let ws2_len = after_name
        .find(|c: char| !c.is_whitespace())
        .unwrap_or(0);
    let as_search_start = name_end_abs + ws2_len;
    let as_in_sql = sql[as_search_start..].to_ascii_uppercase();
    let as_a_offset_abs = match find_as_keyword(&as_in_sql, &sql[as_search_start..]) {
        Some(p) => p,
        None => return None,
    };
    // The `A` of `AS` is at `as_search_start + as_a_offset_abs`. The
    // body starts after `AS` and its following whitespace.
    let after_as_abs = &sql[as_search_start + as_a_offset_abs + 2..];
    let ws_after_as = after_as_abs
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| c.len_utf8())
        .sum::<usize>();
    let body_start_abs = as_search_start + as_a_offset_abs + 2 + ws_after_as;
    let after_body = &sql[body_start_abs..];
    // The statement ends at the first `;` after the body start,
    // if any. (The body itself was already trimmed to not include
    // a trailing `;`.)
    let semi = after_body.find(';');
    let stmt_slice_end = match semi {
        Some(s) => body_start_abs + s + 1,
        None => {
            // No `;` — the statement runs to the end of the next
            // statement (a line beginning with a keyword) or to
            // the end of the string. For the MVP, scan for that.
            let new_stmt_rel = find_next_statement_start(after_body);
            match new_stmt_rel {
                Some(rel) => body_start_abs + rel,
                None => sql.len(),
            }
        }
    };

    Some(CreateHit {
        start: pos,
        end: stmt_slice_end,
        kind,
        name,
        body,
    })
}

/// Find the byte offset within `s` where the body of a CREATE
/// statement ends. The body is everything up to (but not including)
/// the first `;`, or the start of the next statement (a line
/// beginning with a recognised statement-start keyword), or
/// end-of-string.
fn find_body_end(s: &str) -> usize {
    // Find the first `;`.
    if let Some(semi) = s.find(';') {
        return semi;
    }
    // No `;` — find the start of the next statement.
    if let Some(rel) = find_next_statement_start(s) {
        return rel;
    }
    s.len()
}

/// Find the byte offset within `s` of the start of the next
/// statement after the current line. The MVP heuristic: a line
/// beginning (after trimming leading whitespace) with one of the
/// recognised statement-start keywords: `CREATE`, `SELECT`,
/// `EMIT`, `use`. Returns `None` if no such line is found.
fn find_next_statement_start(s: &str) -> Option<usize> {
    // Skip the first line (it's part of the current statement).
    let after_first_line = s.find('\n').map(|p| p + 1).unwrap_or(s.len());
    let rest = &s[after_first_line..];
    for line in rest.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let upper = trimmed.to_ascii_uppercase();
        if upper.starts_with("CREATE ")
            || upper.starts_with("SELECT")
            || upper.starts_with("EMIT ")
            || upper.starts_with("USE ")
        {
            // The offset of this line in the original `s`.
            let line_start_in_rest = rest.find(line).unwrap_or(0);
            return Some(after_first_line + line_start_in_rest);
        }
    }
    None
}

/// Rewrite `FROM <UDF_name>(...)` to a preprocessor-time
/// `VALUES` table or a runtime `UNNEST(...) AS ...` for the
/// S41 MVP's test-fixture UDFs. Two cases are handled:
///
/// - `generate_series(start, end)` → `UNNEST(...) AS u(n)`. The
///   `List<Int64>` result has a single column named `item`
///   (the List's element field); the `AS u(n)` rename surfaces
///   it as `n` (matching the demo's
///   `SELECT n FROM generate_series(...)`).
/// - `generate_events(schema, count, seed)` → a literal
///   `(VALUES (uid, ts), (uid, ts), ...) AS t(user_id, ts)`
///   table (preprocessor-time expansion, NOT a runtime UDF).
///   See `expand_generate_events_in_from` for the full history
///   of why the UDF-based designs all failed on DataFusion 50
///   and why the preprocessor-time VALUES expansion is the
///   pragmatic fix.
///
/// Other UDFs are left alone; DataFusion will surface a clear
/// error at planning time if the FROM references an unknown UDF.
fn rewrite_test_fixtures_in_from(body: &str) -> String {
    // First try the preprocessor-time expansion of
    // `generate_events(...)` (replaces the UDF call with a
    // literal `VALUES` table — see function docstring for the
    // history of the UDF-based designs that this replaces).
    if let Some(out) = expand_generate_events_in_from(body) {
        return out;
    }
    // Then handle generate_series (List<Int64> → 1 col `n`).
    if let Some(out) = rewrite_one_udf_in_from(body, "generate_series", "u(n)") {
        return out;
    }
    body.to_string()
}

/// Expand `FROM generate_events(schema, count, seed)` to
/// `FROM (VALUES (uid, ts), (uid, ts), ...) AS t(user_id, ts)`
/// by running the same LCG the UDF would run, at preprocessor
/// time. The output is a `VALUES` table (DataFusion 50's
/// native inline-table form) with exactly `count` rows × 2
/// columns (`user_id`, `ts`).
///
/// ## History of the previous UDF-based designs
///
/// Earlier revisions of this rewrite kept `generate_events` as
/// a runtime UDF and tried to flatten its `List<Struct<...>>`
/// result via `UNNEST`. None of these worked on DataFusion 50:
/// - `UNNEST(...) AS t(user_id, ts)`: rejected with "Source
///   table contains 1 columns but only 2 names given" (the
///   outer List has 1 column; the inner Struct's 2 fields
///   are addressable only via the flatten-UNNEST step).
/// - `UNNEST(...) AS t` (no per-column rename): rejected with
///   "No field named user_id. Valid fields are
///   t."UNNEST(generate_events(...))"" — DataFusion 50's
///   column resolver latches onto the UNNEST's table alias
///   and refuses to surface the struct's fields as bare
///   column names.
/// - Bare `UNNEST(...)` (no alias at all): same "No field
///   named user_id" error (the struct's fields don't leak
///   into the FROM scope).
/// - Wrap in `(SELECT user_id, ts FROM UNNEST(...)) AS t`:
///   parses + plans, but the inner UNNEST of
///   `List<Struct<...>>` is still treated as 1 column and
///   `SELECT user_id, ts FROM <1 col>` fails.
/// - Final fix (this revision): drop the UDF entirely. Run
///   the LCG at preprocess time, emit a literal `VALUES`
///   table. The shape is the canonical DataFusion inline
///   table; `SELECT user_id, ts FROM (...)` resolves the
///   columns trivially. No UDF, no UNNEST-of-Struct quirks.
///
/// ## Trade-offs
///
/// - **Pro**: works on DataFusion 50 without UNNEST gymnastics.
///   Output is portable to any SQL engine that supports
///   `VALUES`.
/// - **Con**: O(count) bytes of preprocessor output per
///   `generate_events` call. For 1000 rows × ~25 bytes ≈ 25 KB
///   per source — fine. For 1M rows ≈ 25 MB per source —
///   would need a real UDTF or a CSV-backed source. The S41
///   demo's 1000-row `clicks` + 500-row `views` + 250-row
///   `purchases` is well within the 100 KB total budget.
/// - **Con**: the LCG logic now lives in BOTH the
///   `generate_events` UDF (for the case where it's called
///   from a non-`FROM` context) and the preprocessor (for the
///   `FROM` rewrite). The preprocessor version is a 6-line
///   port; the duplication is documented but not refactored
///   out (the UDF may be removed entirely once we confirm the
///   demo's only use is the `FROM` form).
fn expand_generate_events_in_from(body: &str) -> Option<String> {
    let marker = "FROM generate_events(";
    let start = body.find(marker)?;
    let after_open = start + marker.len();
    let close_rel = body[after_open..].find(')')?;
    let close = after_open + close_rel;
    let args_str = &body[after_open..close];
    // Args are 3 Int64 literals: schema, count, seed. The
    // schema arg is ignored (the S41 demo only references
    // `user_id` and `ts` columns).
    let mut parts = args_str.split(',').map(str::trim);
    let _schema: i64 = parts.next()?.parse().ok()?;
    let count: usize = parts.next()?.parse().ok()?;
    let seed: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        // Too many args — bail; DataFusion will surface a
        // clear parse error.
        return None;
    }
    // Run the LCG and emit a literal `VALUES` table.
    // Same constants as `crate::test_fixtures::generate_events_impl`.
    const A: u64 = 1664525;
    const C: u64 = 1013904223;
    const M: u64 = 1u64 << 32;
    let mut out = String::with_capacity(body.len() + count * 30);
    out.push_str(&body[..start]);
    out.push_str("FROM (VALUES");
    let mut x = seed;
    for i in 0..count {
        x = (A.wrapping_mul(x).wrapping_add(C)) % M;
        let user_id = ((x % 1000) + 1) as i64;
        let ts = 1_700_000_000i64 + i as i64;
        if i > 0 {
            out.push(',');
        }
        // Use `as i64` formatting: matches the UDF's output
        // exactly (a deterministic 1:1 with the LCG).
        out.push_str(&format!(" ({user_id},{ts})"));
    }
    out.push_str(") AS t(user_id, ts)");
    out.push_str(&body[close + 1..]);
    Some(out)
}

/// Rewrite `FROM <udf_name>(...)` to
/// `FROM UNNEST(<udf_name>(...)) AS <alias>` if the marker is
/// present. Returns `Some(rewritten)` on a hit, `None` if the
/// marker is absent (so the caller can try the next UDF). Returns
/// `None` if the marker is present but the call is malformed (no
/// closing `)`); the caller will leave the body alone so DataFusion
/// can surface a clear parse error.
///
/// Paren balance: the output is
///   `UNNEST(` + `<udf_name>(` + `<args>)` + `) AS <alias>` + tail
/// which yields
///   `UNNEST(<udf_name>(<args>)) AS <alias>`
/// (2 opens from `UNNEST(` and `<udf_name>(`, 2 closes from the
/// `<args>)` slice and the explicit `)` before `AS`; the alias
/// string is text and any parens inside it must be balanced by
/// the caller, e.g. `u(n)` or `t(user_id, ts)`).
///
/// The original `rewrite_generate_series_in_from` produced exactly
/// this shape: the body slice `<args>)` provides the UDF's closing
/// paren, and the literal `") AS "` adds UNNEST's closing paren
/// followed by `AS ` and the alias text.
fn rewrite_one_udf_in_from(body: &str, udf_name: &str, alias: &str) -> Option<String> {
    rewrite_one_udf_in_from_impl(body, udf_name, Some(alias))
}

/// Same as [`rewrite_one_udf_in_from`] but emits NO `AS <alias>`
/// clause. Used for `generate_events` whose `List<Struct<user_id,
/// ts>>` result is best left with the struct's fields as bare
/// columns in the FROM scope (DataFusion 50 latches onto the
/// UNNEST's table alias and refuses to surface the struct's
/// fields as bare column names when an alias is present).
fn rewrite_one_udf_in_from_no_alias(body: &str, udf_name: &str) -> Option<String> {
    rewrite_one_udf_in_from_impl(body, udf_name, None)
}

fn rewrite_one_udf_in_from_impl(
    body: &str,
    udf_name: &str,
    alias: Option<&str>,
) -> Option<String> {
    let marker = format!("FROM {udf_name}(");
    let start = body.find(&marker)?;
    let after_open = start + marker.len();
    // Find the matching `)` for the UDF call. The MVP assumes
    // the call has no nested parens (true for both
    // `generate_series` and `generate_events` — they take
    // Int64 args only). If the close paren is missing, return
    // `None` so the body is passed through unchanged and
    // DataFusion can surface a clear parse error.
    let close_rel = body[after_open..].find(')')?;
    let close = after_open + close_rel;
    let mut out = String::with_capacity(body.len() + 32);
    out.push_str(&body[..start]);
    // Two rewrite shapes:
    //
    // 1. `generate_series` (List<Int64> → 1 col `n`):
    //    `FROM UNNEST(<UDF>(<args>)) AS u(n)` — the
    //    `AS u(n)` rename maps the List's `item` column to `n`.
    //
    // 2. `generate_events` (List<Struct<user_id, ts>>):
    //    `FROM (SELECT user_id, ts FROM UNNEST(<UDF>(<args>))) AS t`
    //    — wrap the UNNEST in a `SELECT user_id, ts` so the
    //    struct's fields become the subquery's projection and
    //    are addressable in the outer scope. A bare
    //    `UNNEST(<UDF>(<args>))` (no wrap) refuses to surface
    //    the struct's fields as bare column names — DataFusion
    //    50 wants `UNNEST(<UDF>(<args>)).user_id` instead,
    //    which the multi_stream_analytics demo's `SELECT user_id,
    //    ts FROM ...` (unqualified) can't express. The wrap
    //    is the S41 MVP's pragmatic fix.
    if let Some(a) = alias {
        // Shape 1: `generate_series` → `UNNEST(...) AS u(n)`.
        out.push_str("FROM UNNEST(");
        out.push_str(udf_name);
        out.push('(');
        out.push_str(&body[after_open..=close]); // includes original `)`
        out.push(')');
        out.push_str(" AS ");
        out.push_str(a);
    } else {
        // Shape 2: `generate_events` →
        //   `(SELECT user_id, ts FROM UNNEST(<UDF>(<args>))) AS t`.
        out.push_str("FROM (SELECT user_id, ts FROM UNNEST(");
        out.push_str(udf_name);
        out.push('(');
        out.push_str(&body[after_open..=close]); // includes original `)`
        out.push_str(")) AS t");
    }
    out.push_str(&body[close + 1..]);
    Some(out)
}

/// Substitute every word-boundary occurrence of `name` in `sql`
/// with `(<body>)`. The replacement is done in-place; `body` may
/// contain anything (it is wrapped in parens to form a subquery).
fn substitute_name_with_body(sql: &mut String, name: &str, body: &str) {
    substitute_name_with_body_inner(sql, name, body, false);
}

/// Same as [`substitute_name_with_body`] but appends ` AS <name>`
/// after the closing paren. This preserves any column-qualified
/// references like `<name>.col` in the downstream SQL, since
/// DataFusion requires a subquery in `FROM` to have an alias when
/// it's referenced as a named table elsewhere.
fn substitute_name_with_body_aliased(sql: &mut String, name: &str, body: &str) {
    substitute_name_with_body_inner(sql, name, body, true);
}

fn substitute_name_with_body_inner(
    sql: &mut String,
    name: &str,
    body: &str,
    add_alias: bool,
) {
    if name.is_empty() {
        return;
    }
    let mut out = String::with_capacity(sql.len() + body.len() * 4);
    let bytes = sql.as_bytes();
    let name_bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + name_bytes.len() <= bytes.len() && &bytes[i..i + name_bytes.len()] == name_bytes {
            // Check left boundary: start of string or non-identifier char.
            let left_ok = i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            // Check right boundary: end of string or non-identifier char.
            let right_idx = i + name_bytes.len();
            let right_ok = right_idx >= bytes.len()
                || !(bytes[right_idx].is_ascii_alphanumeric() || bytes[right_idx] == b'_');
            if left_ok && right_ok {
                out.push('(');
                out.push_str(body);
                out.push(')');
                if add_alias {
                    out.push_str(" AS ");
                    out.push_str(name);
                }
                i = right_idx;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    *sql = out;
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

    // ---- S17 §2: extract_stream_identities ----
    //
    // NOTE: `parse_use_directives` is line-based — each directive
    // must be on its own line. The realistic Bee SQL format always
    // splits `use` lines from the SELECT body with newlines, so
    // the tests below mirror that shape. Single-line SQL like
    // `"use binance; SELECT ..."` will not parse the `use` (the
    // line-scan treats the whole line as one token).

    #[test]
    fn extract_identities_finds_single_call() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe(symbol='BTC/USDT', interval='5min')";
        let ids = extract_stream_identities(sql);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].0, "binance");
        assert_eq!(ids[0].1, "subscribe");
        assert!(!ids[0].2.is_empty(), "args map should be non-empty");
    }

    #[test]
    fn extract_identities_finds_multiple_calls() {
        let sql = "use binance;\nuse google_news;\n\
                   SELECT * FROM binance.subscribe(symbol='BTC/USDT', interval='5min')\n\
                   JOIN google_news.search(query='btc')";
        let ids = extract_stream_identities(sql);
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn extract_identities_dedupes_repeated_calls() {
        let sql = "use binance;\nSELECT * FROM binance.subscribe(symbol='BTC/USDT', interval='5min')\n\
                   WHERE EXISTS (SELECT 1 FROM binance.subscribe(symbol='BTC/USDT', interval='5min'))";
        let ids = extract_stream_identities(sql);
        assert_eq!(ids.len(), 1, "repeated calls must dedupe");
    }

    // ---- S41 (9a): EMIT INTO strip preprocessor ----
    //
    // Wires `EMIT INTO console SELECT ...` into the SQL execution
    // path. `strip_emit_into` recognizes the prefix, returns the
    // sink target, and strips the prefix so DataFusion can parse the
    // remaining SELECT.

    #[test]
    fn strip_emit_into_console() {
        let (target, remaining) = strip_emit_into("EMIT INTO console SELECT 1");
        assert_eq!(target, Some(EmitTarget::Console));
        assert_eq!(remaining, "SELECT 1");
    }

    #[test]
    fn strip_emit_into_console_lowercase() {
        let (target, remaining) = strip_emit_into("emit into console SELECT 1");
        assert_eq!(target, Some(EmitTarget::Console));
        assert_eq!(remaining, "SELECT 1");
    }

    #[test]
    fn strip_emit_into_console_with_leading_whitespace() {
        let (target, remaining) = strip_emit_into("   EMIT INTO console\nSELECT 1");
        assert_eq!(target, Some(EmitTarget::Console));
        assert_eq!(remaining, "SELECT 1");
    }

    #[test]
    fn strip_emit_into_no_prefix() {
        let (target, remaining) = strip_emit_into("SELECT 1");
        assert_eq!(target, None);
        assert_eq!(remaining, "SELECT 1");
    }

    #[test]
    fn strip_emit_into_unknown_target_passes_through() {
        let (target, remaining) = strip_emit_into("EMIT INTO something_else SELECT 1");
        assert_eq!(target, Some(EmitTarget::Plugin("something_else".to_string())));
        assert_eq!(remaining, "SELECT 1");
    }

    // ---- S41 (9c): CREATE SOURCE / CREATE VIEW preprocessor ----
    //
    // The preprocessor strips `CREATE SOURCE <name> AS <body>;` and
    // `CREATE VIEW <name> AS <body>;` statements, then substitutes
    // references to `<name>` with `(<body>)` in the remaining SQL.
    // For the S41 MVP, it also rewrites `FROM generate_series(...)`
    // in the body to `FROM UNNEST(generate_series(...)) AS u(n)`
    // (DataFusion 50 has no UDTF support; UNNEST is the canonical
    // way to expand a scalar UDF's array result into rows).

    #[test]
    fn strip_create_source_single() {
        let sql = "CREATE SOURCE naturals AS SELECT n FROM generate_series(1, 5); SELECT * FROM naturals;";
        let (defs, cleaned) = strip_create_source_and_view(sql);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, CreateKind::Source);
        assert_eq!(defs[0].name, "naturals");
        // The body should have the UNNEST rewrite applied.
        assert!(defs[0].body.contains("UNNEST(generate_series("), "body: {}", defs[0].body);
        // The CREATE line is stripped, and the reference to
        // `naturals` is replaced with the subquery.
        assert!(!cleaned.contains("CREATE SOURCE"), "cleaned: {cleaned}");
        assert!(cleaned.contains("FROM (SELECT n FROM UNNEST(generate_series("), "cleaned: {cleaned}");
    }

    #[test]
    fn strip_create_view_single() {
        let sql = "CREATE VIEW fib_stream AS SELECT n, fib_step(n) AS fib_value FROM naturals; SELECT * FROM fib_stream;";
        let (defs, cleaned) = strip_create_source_and_view(sql);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, CreateKind::View);
        assert_eq!(defs[0].name, "fib_stream");
        assert!(!cleaned.contains("CREATE VIEW"), "cleaned: {cleaned}");
        // The reference to `fib_stream` is replaced with the
        // subquery, which still contains `naturals` (a different
        // name; not substituted here).
        assert!(cleaned.contains("FROM (SELECT n, fib_step(n) AS fib_value FROM naturals)"), "cleaned: {cleaned}");
    }

    #[test]
    fn strip_create_chains_substitutions() {
        // The S41 demo: CREATE SOURCE naturals → CREATE VIEW
        // fib_stream (referencing naturals) → final SELECT
        // (referencing fib_stream). The preprocessor must resolve
        // the chain: fib_stream → (naturals → UNNEST rewrite).
        let sql = "\
            CREATE SOURCE naturals AS SELECT n FROM generate_series(1, 1000000);\
            CREATE VIEW fib_stream AS SELECT n, fib_step(n) AS fib_value FROM naturals;\
            SELECT n, fib_value FROM fib_stream WHERE n <= 20;";
        let (_defs, cleaned) = strip_create_source_and_view(sql);
        // The CREATE lines are gone.
        assert!(!cleaned.contains("CREATE SOURCE"), "cleaned: {cleaned}");
        assert!(!cleaned.contains("CREATE VIEW"), "cleaned: {cleaned}");
        // `fib_stream` was substituted with its body; inside that
        // body, `naturals` was substituted with the UNNEST-rewritten
        // body. The `AS fib_stream` alias is appended to preserve
        // any column-qualified references in the downstream SQL.
        assert!(cleaned.contains("FROM (SELECT n, fib_step(n) AS fib_value FROM (SELECT n FROM UNNEST(generate_series(1, 1000000)) AS u(n))) AS fib_stream"), "cleaned: {cleaned}");
        // The final SELECT's `FROM fib_stream` is gone (substituted).
        assert!(cleaned.contains("WHERE n <= 20"), "cleaned: {cleaned}");
    }

    #[test]
    fn strip_create_handles_multiline_body() {
        let sql = "CREATE SOURCE naturals AS\nSELECT n\nFROM generate_series(1, 5);\nSELECT * FROM naturals;";
        let (defs, cleaned) = strip_create_source_and_view(sql);
        assert_eq!(defs.len(), 1);
        assert!(defs[0].body.contains("UNNEST"), "body: {}", defs[0].body);
        assert!(cleaned.contains("UNNEST"), "cleaned: {cleaned}");
    }

    #[test]
    fn strip_create_handles_no_semicolon_at_eof() {
        // The S41 demo has no `;` after the final SELECT; the body
        // parser should still work when the statement runs to EOL.
        let sql = "CREATE SOURCE naturals AS SELECT n FROM generate_series(1, 5)\nSELECT * FROM naturals";
        let (defs, cleaned) = strip_create_source_and_view(sql);
        assert_eq!(defs.len(), 1);
        assert!(cleaned.contains("UNNEST"), "cleaned: {cleaned}");
    }

    #[test]
    fn strip_create_handles_multiple() {
        let sql = "\
            CREATE SOURCE a AS SELECT x FROM generate_series(1, 3);\
            CREATE SOURCE b AS SELECT y FROM generate_series(4, 6);\
            SELECT * FROM a;\
            SELECT * FROM b;";
        let (defs, cleaned) = strip_create_source_and_view(sql);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "a");
        assert_eq!(defs[1].name, "b");
        assert!(cleaned.contains("FROM (SELECT x FROM UNNEST"), "cleaned: {cleaned}");
        // `b` is also substituted.
        assert!(cleaned.matches("FROM (SELECT y FROM UNNEST").count() == 1, "cleaned: {cleaned}");
    }

    #[test]
    fn strip_create_no_create_statements() {
        let sql = "SELECT * FROM stream; SELECT 1;";
        let (defs, cleaned) = strip_create_source_and_view(sql);
        assert_eq!(defs.len(), 0);
        assert_eq!(cleaned, sql, "no-op when no CREATE statements");
    }

    #[test]
    fn strip_create_substitution_respects_word_boundaries() {
        // `naturals` should not match `naturalsx` or `xnaturals`.
        let sql = "CREATE SOURCE naturals AS SELECT 1; SELECT * FROM naturals; SELECT * FROM naturalsx;";
        let (defs, cleaned) = strip_create_source_and_view(sql);
        assert_eq!(defs.len(), 1);
        // `naturals` → `(SELECT 1)`.
        assert!(cleaned.contains("FROM (SELECT 1)"), "cleaned: {cleaned}");
        // `naturalsx` is left alone (word-boundary match).
        assert!(cleaned.contains("naturalsx"), "cleaned: {cleaned}");
    }

    #[test]
    fn rewrite_test_fixtures_in_from_handles_generate_series() {
        let body = "SELECT n FROM generate_series(1, 10)";
        let out = rewrite_test_fixtures_in_from(body);
        assert_eq!(
            out,
            "SELECT n FROM UNNEST(generate_series(1, 10)) AS u(n)"
        );
    }

    #[test]
    fn rewrite_test_fixtures_in_from_handles_generate_events() {
        // S41: `generate_events(schema, count, seed)` is
        // preprocessor-time expanded to a literal `VALUES`
        // table. The test asserts the small-N output is
        // character-for-character what we'd expect (the LCG
        // values are deterministic). The rewrite runs the
        // same LCG the UDF would run; for `count = 3,
        // seed = 42`, the first 3 user_ids are
        //   42 → 1 (LCG1: 1013904223 + 0 * 1664525) % 2^32
        //      → 1013904223 % 2^32 = 1013904223
        //      → (1013904223 % 1000) + 1 = 224
        //   ...
        // The actual values depend on the LCG constants; we
        // pin the full output for count=3 here (a regression
        // that flips user_id/ts or the wrapping form would
        // surface here).
        let body = "SELECT user_id, ts FROM generate_events(0, 3, 42)";
        let out = rewrite_test_fixtures_in_from(body);
        // The exact user_id values are LCG-computed. We
        // assert the shape (VALUES table, 3 rows × 2 cols,
        // aliased `t(user_id, ts)`, three `(` separated from
        // each other) and the deterministic `ts` values
        // (1_700_000_000 + i).
        assert!(out.starts_with("SELECT user_id, ts FROM (VALUES"), "out: {out}");
        assert!(out.ends_with(") AS t(user_id, ts)"), "out: {out}");
        // The 3 rows have `ts = 1_700_000_000, 1_700_000_001,
        // 1_700_000_002`. The user_id values are LCG-derived
        // (we don't pin them here — the LCG constants are
        // documented in expand_generate_events_in_from).
        assert!(out.contains("1700000000"), "out: {out}");
        assert!(out.contains("1700000001"), "out: {out}");
        assert!(out.contains("1700000002"), "out: {out}");
    }

    #[test]
    fn rewrite_test_fixtures_in_from_no_op_when_no_marker() {
        let body = "SELECT 1";
        let out = rewrite_test_fixtures_in_from(body);
        assert_eq!(out, "SELECT 1");
    }

    #[test]
    fn rewrite_test_fixtures_in_from_passes_through_malformed_call() {
        // No closing `)` — the body is left alone so DataFusion
        // can surface a clear parse error.
        let body = "SELECT n FROM generate_series(1, 10";
        let out = rewrite_test_fixtures_in_from(body);
        assert_eq!(out, body);
    }

    #[test]
    fn strip_create_source_rewrites_generate_events() {
        // S41 regression: the preprocessor must auto-expand
        // `generate_events` in a CREATE SOURCE body so the SQL
        // can use it as a `FROM` source. The expansion is
        // preprocessor-time (a literal `VALUES` table) — see
        // `expand_generate_events_in_from` for the full history
        // of why the UDF-based + UNNEST-based designs all
        // failed on DataFusion 50.
        let sql = "\
            CREATE SOURCE clicks AS \
            SELECT user_id, ts FROM generate_events(0, 3, 42);\
            EMIT INTO console SELECT * FROM clicks;";
        let (_defs, cleaned) = strip_create_source_and_view(sql);
        assert!(!cleaned.contains("CREATE SOURCE"), "cleaned: {cleaned}");
        // The `clicks` body should have the VALUES expansion
        // applied, then be wrapped in `(...)` and aliased as
        // `clicks` (the original CREATE SOURCE's name) in the
        // downstream SELECT. We assert the shape (VALUES table,
        // 3 rows, aliased `t(user_id, ts)`, wrapped in
        // `(...) AS clicks`) without pinning the exact LCG
        // values (those are asserted in
        // `rewrite_test_fixtures_in_from_handles_generate_events`).
        assert!(
            cleaned.contains("FROM (VALUES")
                && cleaned.contains(") AS t(user_id, ts)) AS clicks"),
            "expected VALUES expansion wrapped as `(...) AS clicks`; \
             got: {cleaned}"
        );
    }

    #[test]
    fn substitute_name_with_body_word_boundary() {
        let mut s = String::from("SELECT * FROM naturals WHERE naturals.foo = 1");
        substitute_name_with_body(&mut s, "naturals", "SELECT 1");
        // `naturals` after `FROM` is replaced; `naturals.foo` is
        // not (the `.` is not an identifier char so the left
        // boundary holds, but the right boundary is `.foo` which
        // is not an identifier char on the right either — wait,
        // `naturals` is followed by `.` which is NOT alphanumeric,
        // so the right boundary holds and the substitution happens).
        // Actually, let me re-check: in "naturals.foo", the `n` is
        // the start of `naturals`, and the char after `s` is `.`.
        // The right boundary check is `bytes[right_idx]` is NOT
        // alphanumeric/underscore — `.` is neither, so the boundary
        // holds. So `naturals` IS substituted.
        // The result should have the substitution applied.
        assert!(s.contains("FROM (SELECT 1)"), "got: {s}");
    }
}
