//! Pure-function line-protocol encoder.
//!
//! The encoder takes a [`LineProtocolRow`] (an internal,
// pre-decoded struct) and emits the v2 line protocol string.
//!
//! The `Event` -> `LineProtocolRow` translation lives in
//! [`crate::write`]; this module is the pure encoder so it can be
//! unit-tested in isolation.
//!
//! ## Spec
//!
//! <https://docs.influxdata.com/influxdb/v2/reference/syntax/line-protocol/>
//!
//! ```text
//! measurement,tag1=v1,tag2=v2 field1=42i,field2="hello" 1700000000000000000
//! ```
//!
//! - Measurement and tag / field keys are passed through
//!   [`crate::escape::escape`].
//! - Tag values are passed through [`crate::escape::escape`].
//! - Field values are already typed strings (the caller
//!   pre-formats them as `"42i"`, `"3.14"`, `"true"`, `"\"hello\""`).
//! - Timestamp is the bare `i64` nanoseconds string.

use std::collections::HashMap;

use crate::escape::escape;

/// One row of v2 line protocol, in the form we hand to the
/// encoder. The `fields` map's values are ALREADY TYPED strings
/// (`"42i"`, `"3.14"`, `"true"`, `"\"hello\""`); the encoder
/// does no type detection.
#[derive(Debug, Clone, PartialEq)]
pub struct LineProtocolRow {
    pub measurement: String,
    pub tags: HashMap<String, String>,
    pub fields: HashMap<String, String>,
    pub timestamp_ns: i64,
}

impl LineProtocolRow {
    /// Build an empty row with the given measurement and
    /// timestamp. Tags and fields are added via the maps.
    pub fn new(measurement: impl Into<String>, timestamp_ns: i64) -> Self {
        Self {
            measurement: measurement.into(),
            tags: HashMap::new(),
            fields: HashMap::new(),
            timestamp_ns,
        }
    }
}

/// Encode a single [`LineProtocolRow`] into the v2 line protocol.
///
/// Format: `<measurement>[,tagk=tagv...] <fieldk=fieldv...> <timestamp>`.
///
/// - Tags are emitted in sorted key order for determinism
///   (InfluxDB v2 accepts any order, but sorted output makes
///   the bytes easier to test).
/// - Fields are emitted in sorted key order for the same reason.
pub fn encode_line_protocol(row: &LineProtocolRow) -> String {
    let mut s = String::new();
    s.push_str(&escape(&row.measurement));

    if !row.tags.is_empty() {
        s.push(',');
        let mut tag_keys: Vec<&String> = row.tags.keys().collect();
        tag_keys.sort();
        let parts: Vec<String> = tag_keys
            .into_iter()
            .map(|k| {
                let v = &row.tags[k];
                format!("{}={}", escape(k), escape(v))
            })
            .collect();
        s.push_str(&parts.join(","));
    }

    s.push(' ');
    {
        let mut field_keys: Vec<&String> = row.fields.keys().collect();
        field_keys.sort();
        let parts: Vec<String> = field_keys
            .into_iter()
            .map(|k| {
                let v = &row.fields[k];
                format!("{}={}", escape(k), v)
            })
            .collect();
        s.push_str(&parts.join(","));
    }

    s.push(' ');
    s.push_str(&row.timestamp_ns.to_string());
    s
}
