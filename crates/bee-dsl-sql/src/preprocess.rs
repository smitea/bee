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

use bee_plugin_sdk::VersionSpec;

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
}
