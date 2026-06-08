//! S17 §1: canonical hash of a Stream's identity (ADR-0011).
//!
//! `StreamSignature = hex(sha256(name || ":" || method || ":" || inner))`
//! where `inner = hex(sha256(canonical_json(stream_topology_args)))`.
//!
//! BTreeMap serialization is key-sorted, so `serde_json::to_string`
//! is canonical for `BTreeMap<String, String>`.
//!
//! This module is pure: no I/O, no async, no Bee types. Anything
//! that needs to fingerprint a Stream — deployer, control plane,
//! jobs view — funnels through this single function.

use std::collections::BTreeMap;
use sha2::{Digest, Sha256};

/// Compute the StreamSignature for a given (datasource, method, args)
/// triple per ADR-0011. Returns 64 lowercase hex chars.
pub fn stream_signature(
    datasource_name: &str,
    adapter_method: &str,
    stream_topology_args: &BTreeMap<String, String>,
) -> String {
    // inner: sha256 over the canonical JSON of the topology args.
    // BTreeMap<String, String> serializes in key-sorted order, so
    // serde_json::to_string is canonical for free.
    let inner = {
        let json = serde_json::to_string(stream_topology_args)
            .expect("BTreeMap<String, String> is always serializable");
        hex::encode(Sha256::digest(json.as_bytes()))
    };
    // outer: sha256 over `name ":" method ":" inner`
    let mut h = Sha256::new();
    h.update(datasource_name.as_bytes());
    h.update(b":");
    h.update(adapter_method.as_bytes());
    h.update(b":");
    h.update(inner.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn deterministic_for_same_inputs() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "5min")]));
        let b = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "5min")]));
        assert!(!a.is_empty(), "must produce a non-empty signature");
        assert_eq!(a, b, "same inputs must hash to the same value");
    }

    #[test]
    fn different_datasource_yields_different_signature() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT")]));
        let b = stream_signature("google_news", "search",
            &args(&[("query", "btc")]));
        assert_ne!(a, b);
        assert!(!a.is_empty() && !b.is_empty(),
            "both must produce non-empty signatures");
    }

    #[test]
    fn different_method_yields_different_signature() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT")]));
        let b = stream_signature("binance", "emit",
            &args(&[("symbol", "BTC/USDT")]));
        assert_ne!(a, b);
    }

    #[test]
    fn different_args_yield_different_signature() {
        let a = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "5min")]));
        let b = stream_signature("binance", "subscribe",
            &args(&[("symbol", "ETH/USDT"), ("interval", "5min")]));
        let c = stream_signature("binance", "subscribe",
            &args(&[("symbol", "BTC/USDT"), ("interval", "1min")]));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn empty_args_yields_valid_signature() {
        let s = stream_signature("binance", "ping", &BTreeMap::new());
        // 64 hex chars = 32 bytes = sha256 output
        assert_eq!(s.len(), 64,
            "sha256 hex must be 64 chars, got {} chars: {s:?}", s.len());
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()),
            "must be lowercase hex, got: {s:?}");
    }

    #[test]
    fn arg_order_does_not_matter() {
        // BTreeMap gives the same key order regardless of insertion order.
        let mut a = BTreeMap::new();
        a.insert("symbol".to_string(), "BTC/USDT".to_string());
        a.insert("interval".to_string(), "5min".to_string());
        let mut b = BTreeMap::new();
        b.insert("interval".to_string(), "5min".to_string());
        b.insert("symbol".to_string(), "BTC/USDT".to_string());
        let sa = stream_signature("binance", "subscribe", &a);
        let sb = stream_signature("binance", "subscribe", &b);
        assert_eq!(sa, sb);
    }
}
