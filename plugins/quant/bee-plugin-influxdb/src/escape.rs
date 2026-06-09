//! InfluxDB v2 line-protocol escaping.
//!
//! ## Spec reference
//!
//! <https://docs.influxdata.com/influxdb/v2/reference/syntax/line-protocol/>
//!
//! ### Measurement
//!
//! Must escape `,` and ` ` with a backslash.
//!
//! ### Tag keys / tag values
//!
//! Tag keys: must escape `,`, `=`, ` ` with a backslash.
//! Tag values: must escape `,`, `=`, ` ` with a backslash.
//!
//! ### Field keys
//!
//! Must escape `,`, `=`, ` ` with a backslash.
//!
//! ### Field values
//!
//! Field values are typed at the call site; this module does not
//! escape the user-supplied value (the plugin's own typed
//! encoding handles it — strings are quoted, numbers are bare).
//!
//! ### Timestamp
//!
//! Always bare integer. The plugin's encoder emits a plain
//! `i64.to_string()`.

/// Escape a measurement, tag key, tag value, or field key.
///
/// The same escaping rules apply to all four: replace `,`, `=`,
/// and ` ` with their backslash-prefixed forms. The InfluxDB v2
/// line protocol requires this; the upstream parser rejects
/// unescaped separators.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            ',' | '=' | ' ' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}
