//! `bee-plugin-ta-lib` — production-grade reference implementation.
//!
//! Implements Handler UDFs (pure-compute functions invoked by
//! Phases): `ema`, `macd`, `decision_tree`, `sentiment_analyzer`.
//! Plugin scaffold is production-grade (cdylib + FFI vtable); the
//! actual algorithms are simplified placeholder versions that
//! will be replaced by a real `ta-lib` binding in S38.
//!
//! Handlers are free functions with stable signatures. The
//! runtime parses the generic `Event` payload bytes (from
//! `bee-adapter`) and dispatches to the appropriate handler based
//! on the name declared in the plugin's `PluginManifest`.
//!
//! This plugin declares no Adapters — only Handlers.
//!
//! ## Architecture
//!
//! - [`TaLibFactory`]: produces the
//!   [`bee_plugin_sdk::PluginManifest`] + [`bee_plugin_sdk::PluginHandle`]
//!   for the host.
//! - Free-function handlers: the actual compute primitives. Pure
//!   functions; no I/O, no plugin state.
//! - `cdylib_plugin!(Factory)` invocation at the bottom generates
//!   the FFI entry symbols.

use bee_plugin_sdk::{
    event::EventBytes, Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName,
};

/// Result of a MACD computation. The last
/// `(macd_line, signal_line, histogram)` triple.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MacdResult {
    pub macd_line: f64,
    pub signal_line: f64,
    pub histogram: f64,
}

/// Trading decision produced by [`decision_tree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Decision {
    Buy,
    Sell,
    Hold,
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Decision::Buy => f.write_str("buy"),
            Decision::Sell => f.write_str("sell"),
            Decision::Hold => f.write_str("hold"),
        }
    }
}

/// Exponential moving average. Returns the last EMA value.
///
/// `ema[i] = alpha * prices[i] + (1 - alpha) * ema[i-1]`,
/// where `alpha = 2 / (period + 1)`. Seeded with the SMA of the
/// first `period` values. If `prices.len() < period`, returns the
/// arithmetic mean of the input.
pub fn ema(prices: &[f64], period: usize) -> f64 {
    if prices.is_empty() {
        return 0.0;
    }
    if prices.len() < period {
        let sum: f64 = prices.iter().sum();
        return sum / prices.len() as f64;
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let seed: f64 = prices[..period].iter().sum::<f64>() / period as f64;
    let mut prev = seed;
    for &p in &prices[period..] {
        prev = alpha * p + (1.0 - alpha) * prev;
    }
    prev
}

/// Moving Average Convergence Divergence. Returns the last
/// `(macd_line, signal_line, histogram)` triple.
///
/// `macd_line[i] = ema(prices, fast_period, i) - ema(prices, slow_period, i)`.
/// `signal_line = ema(macd_line, signal_period)` over the last
/// `signal_period` values of `macd_line`.
/// `histogram = macd_line - signal_line`.
pub fn macd(
    prices: &[f64],
    fast_period: usize,
    slow_period: usize,
    signal_period: usize,
) -> MacdResult {
    let fast = ema_series(prices, fast_period);
    let slow = ema_series(prices, slow_period);
    let macd_line: Vec<f64> = fast
        .iter()
        .zip(slow.iter())
        .map(|(f, s)| f - s)
        .collect();
    let signal_line = ema_last_n(&macd_line, signal_period);
    let last_macd = *macd_line.last().unwrap_or(&0.0);
    MacdResult {
        macd_line: last_macd,
        signal_line,
        histogram: last_macd - signal_line,
    }
}

/// Decision tree combining a MACD histogram and a sentiment
/// score.
///
/// - `hist > 0` AND `sentiment > 0.5` → `Buy`
/// - `hist < 0` AND `sentiment < -0.5` → `Sell`
/// - otherwise → `Hold`
pub fn decision_tree(macd_histogram: f64, sentiment_score: f64) -> Decision {
    if macd_histogram > 0.0 && sentiment_score > 0.5 {
        Decision::Buy
    } else if macd_histogram < 0.0 && sentiment_score < -0.5 {
        Decision::Sell
    } else {
        Decision::Hold
    }
}

/// Keyword-based sentiment analyzer. Returns a score in
/// `[-1, 1]`.
///
/// Positive keywords: `bullish`, `growth`, `high`, `buy`, `rally`,
/// `adoption`. Negative keywords: `bearish`, `crash`, `low`, `sell`,
/// `decline`, `regulation`. Score = `(positive - negative) / max(positive + negative, 1)`,
/// clamped to `[-1, 1]`. Returns `0.0` when no keywords match.
pub fn sentiment_analyzer(text: &str) -> f64 {
    const POSITIVE: &[&str] = &[
        "bullish", "growth", "high", "buy", "rally", "adoption",
    ];
    const NEGATIVE: &[&str] = &[
        "bearish", "crash", "low", "sell", "decline", "regulation",
    ];
    let positive = POSITIVE.iter().filter(|k| text.contains(*k)).count();
    let negative = NEGATIVE.iter().filter(|k| text.contains(*k)).count();
    let denom = (positive + negative).max(1) as f64;
    let score = (positive as f64 - negative as f64) / denom;
    score.clamp(-1.0, 1.0)
}

/// Compute the full EMA series. `series[i]` is the EMA of
/// `prices[0..=i]`. Returns a `Vec` of the same length as `prices`.
/// The first `period` entries equal the SMA seed.
fn ema_series(prices: &[f64], period: usize) -> Vec<f64> {
    if prices.is_empty() {
        return vec![];
    }
    if prices.len() < period {
        let mean: f64 = prices.iter().sum::<f64>() / prices.len() as f64;
        return vec![mean; prices.len()];
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let seed: f64 = prices[..period].iter().sum::<f64>() / period as f64;
    let mut series = Vec::with_capacity(prices.len());
    for _ in 0..period {
        series.push(seed);
    }
    let mut prev = seed;
    for &p in &prices[period..] {
        prev = alpha * p + (1.0 - alpha) * prev;
        series.push(prev);
    }
    series
}

/// EMA of the last `n` values of `series`. If `series.len() < n`,
/// returns the EMA of the whole series (which falls back to the
/// mean per [`ema`]'s short-input rule).
fn ema_last_n(series: &[f64], n: usize) -> f64 {
    if series.is_empty() {
        return 0.0;
    }
    let n = n.min(series.len());
    if n == 0 {
        return 0.0;
    }
    ema(&series[series.len() - n..], n)
}

/// Factory for the ta-lib plugin. Holds no state; both methods
/// are pure.
pub struct TaLibFactory;

impl Factory for TaLibFactory {
    fn manifest() -> PluginManifest {
        PluginManifest {
            name: PluginName("ta-lib".into()),
            feature_version: "1.0.0".into(),
            abi_version: "v1".into(),
            adapters: vec![],
            handlers: vec![
                HandlerDescriptor { name: "MACD".into() },
                HandlerDescriptor { name: "EMA".into() },
                HandlerDescriptor { name: "decision_tree".into() },
                HandlerDescriptor { name: "sentiment_analyzer".into() },
            ],
        }
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        let mut handlers = std::collections::HashMap::new();
        handlers.insert(
            "MACD".to_string(),
            &vtable_shim_macd::VTABLE as *const _,
        );
        handlers.insert(
            "EMA".to_string(),
            &vtable_shim_ema::VTABLE as *const _,
        );
        handlers.insert(
            "decision_tree".to_string(),
            &vtable_shim_decision_tree::VTABLE as *const _,
        );
        handlers.insert(
            "sentiment_analyzer".to_string(),
            &vtable_shim_sentiment_analyzer::VTABLE as *const _,
        );
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
            input_adapters: std::collections::HashMap::new(),
            output_adapters: std::collections::HashMap::new(),
            handlers,
        })
    }
}

fn write_event_bytes(out: *mut EventBytes, value: impl serde::Serialize) -> i32 {
    let bytes = match bincode::serialize(&value) {
        Ok(b) => b,
        Err(_) => return -1,
    };
    let len = bytes.len();
    let ptr = bytes.as_ptr();
    std::mem::forget(bytes);
    unsafe { *out = EventBytes { ptr, len } };
    0
}

mod vtable_shim_macd {
    //! MACD handler shim. State: `Vec<f64>` (price history).
    //! Event: `f64` (new price). Result: `MacdResult`.

    use super::{macd, MacdResult};

    use bee_plugin_sdk::event::EventBytes;
    use bee_plugin_sdk::vtable::HandlerVtable;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        super::write_event_bytes(out, Vec::<f64>::new())
    }

    pub unsafe extern "C" fn handle(
        state_ptr: *const u8,
        state_len: usize,
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        let state_bytes = std::slice::from_raw_parts(state_ptr, state_len);
        let mut state: Vec<f64> = match bincode::deserialize(state_bytes) {
            Ok(s) => s,
            Err(_) => return -1,
        };
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let price: f64 = match bincode::deserialize(event_bytes) {
            Ok(p) => p,
            Err(_) => return -1,
        };
        state.push(price);
        let result = if state.len() >= 26 {
            macd(&state, 12, 26, 9)
        } else {
            MacdResult {
                macd_line: 0.0,
                signal_line: 0.0,
                histogram: 0.0,
            }
        };
        if super::write_event_bytes(new_state_out, &state) != 0 {
            return -1;
        }
        super::write_event_bytes(result_out, &result)
    }

    pub const VTABLE: HandlerVtable = HandlerVtable {
        handle,
        init_state,
    };
}

mod vtable_shim_ema {
    //! EMA handler shim. State: `Vec<f64>` (price history).
    //! Event: `f64` (new price). Result: `f64` (last EMA value).

    use super::ema;

    use bee_plugin_sdk::event::EventBytes;
    use bee_plugin_sdk::vtable::HandlerVtable;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        super::write_event_bytes(out, Vec::<f64>::new())
    }

    pub unsafe extern "C" fn handle(
        state_ptr: *const u8,
        state_len: usize,
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        let state_bytes = std::slice::from_raw_parts(state_ptr, state_len);
        let mut state: Vec<f64> = match bincode::deserialize(state_bytes) {
            Ok(s) => s,
            Err(_) => return -1,
        };
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let price: f64 = match bincode::deserialize(event_bytes) {
            Ok(p) => p,
            Err(_) => return -1,
        };
        state.push(price);
        let result = ema(&state, 20);
        if super::write_event_bytes(new_state_out, &state) != 0 {
            return -1;
        }
        super::write_event_bytes(result_out, &result)
    }

    pub const VTABLE: HandlerVtable = HandlerVtable {
        handle,
        init_state,
    };
}

mod vtable_shim_decision_tree {
    //! Decision-tree handler shim. State: empty.
    //! Event: `(f64, f64)` = (macd_histogram, sentiment_score).
    //! Result: `Decision`.

    use super::{decision_tree, Decision};

    use bee_plugin_sdk::event::EventBytes;
    use bee_plugin_sdk::vtable::HandlerVtable;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        super::write_event_bytes(out, Vec::<u8>::new())
    }

    pub unsafe extern "C" fn handle(
        _state_ptr: *const u8,
        _state_len: usize,
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let (hist, sentiment): (f64, f64) = match bincode::deserialize(event_bytes) {
            Ok(v) => v,
            Err(_) => return -1,
        };
        let result = decision_tree(hist, sentiment);
        if super::write_event_bytes(new_state_out, Vec::<u8>::new()) != 0 {
            return -1;
        }
        super::write_event_bytes(result_out, &result)
    }

    pub const VTABLE: HandlerVtable = HandlerVtable {
        handle,
        init_state,
    };
}

mod vtable_shim_sentiment_analyzer {
    //! Sentiment-analyzer handler shim. State: empty.
    //! Event: `String` (text to analyze). Result: `f64` in `[-1, 1]`.

    use super::sentiment_analyzer;

    use bee_plugin_sdk::event::EventBytes;
    use bee_plugin_sdk::vtable::HandlerVtable;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        super::write_event_bytes(out, Vec::<u8>::new())
    }

    pub unsafe extern "C" fn handle(
        _state_ptr: *const u8,
        _state_len: usize,
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let text: String = match bincode::deserialize(event_bytes) {
            Ok(s) => s,
            Err(_) => return -1,
        };
        let result = sentiment_analyzer(&text);
        if super::write_event_bytes(new_state_out, Vec::<u8>::new()) != 0 {
            return -1;
        }
        super::write_event_bytes(result_out, &result)
    }

    pub const VTABLE: HandlerVtable = HandlerVtable {
        handle,
        init_state,
    };
}

bee_plugin_sdk::cdylib_plugin!(TaLibFactory);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ema_constant_input_returns_constant() {
        let prices = vec![5.0; 10];
        let result = ema(&prices, 3);
        assert!((result - 5.0).abs() < 1e-9, "expected 5.0, got {result}");
    }

    #[test]
    fn ema_short_input_returns_mean() {
        let prices = vec![1.0, 2.0, 3.0];
        let result = ema(&prices, 5);
        assert!((result - 2.0).abs() < 1e-9, "expected 2.0, got {result}");
    }

    #[test]
    fn ema_increasing_input_trends_up() {
        let prices: Vec<f64> = (1..=10).map(|x| x as f64).collect();
        let result = ema(&prices, 3);
        assert!(result > 3.0, "expected > 3.0, got {result}");
    }

    #[test]
    fn macd_returns_last_triple() {
        let prices: Vec<f64> = (1..=30).map(|x| x as f64).collect();
        let result = macd(&prices, 12, 26, 9);
        assert!(result.macd_line.is_finite());
        assert!(result.signal_line.is_finite());
        assert!(result.histogram.is_finite());
        assert!(result.macd_line > 0.0);
    }

    #[test]
    fn decision_tree_bullish_buy() {
        assert_eq!(decision_tree(0.1, 0.6), Decision::Buy);
    }

    #[test]
    fn decision_tree_bearish_sell() {
        assert_eq!(decision_tree(-0.1, -0.6), Decision::Sell);
    }

    #[test]
    fn decision_tree_mixed_hold() {
        assert_eq!(decision_tree(0.1, -0.6), Decision::Hold);
    }

    #[test]
    fn sentiment_analyzer_all_positive() {
        let text = "bullish growth high buy rally adoption";
        let score = sentiment_analyzer(text);
        assert!((score - 1.0).abs() < 1e-9, "expected 1.0, got {score}");
    }

    #[test]
    fn sentiment_analyzer_all_negative() {
        let text = "bearish crash low sell decline regulation";
        let score = sentiment_analyzer(text);
        assert!((score + 1.0).abs() < 1e-9, "expected -1.0, got {score}");
    }

    #[test]
    fn sentiment_analyzer_neutral() {
        let text = "the weather is nice today";
        let score = sentiment_analyzer(text);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn factory_manifest_declares_four_handlers() {
        let m = TaLibFactory::manifest();
        assert_eq!(m.name.0, "ta-lib");
        assert_eq!(m.abi_version, "v1");
        assert!(m.adapters.is_empty());
        assert_eq!(m.handlers.len(), 4);
        let names: Vec<&str> = m.handlers.iter().map(|h| h.name.as_str()).collect();
        assert!(names.contains(&"MACD"));
        assert!(names.contains(&"EMA"));
        assert!(names.contains(&"decision_tree"));
        assert!(names.contains(&"sentiment_analyzer"));
    }

    #[test]
    fn factory_init_returns_handle_with_manifest() {
        let h = TaLibFactory::init().unwrap();
        assert_eq!(h.manifest.name.0, "ta-lib");
        assert_eq!(h.manifest.handlers.len(), 4);
    }

    #[test]
    fn vtable_macd_init_state_and_handle() {
        let handle = TaLibFactory::init().expect("init");
        let vtable = *handle.handlers.get("MACD").expect("MACD vtable");
        let mut state_out = EventBytes::EMPTY;
        let rc = unsafe { ((*vtable).init_state)(&mut state_out) };
        assert_eq!(rc, 0);
        let state: Vec<f64> =
            bincode::deserialize(unsafe { std::slice::from_raw_parts(state_out.ptr, state_out.len) })
                .expect("decode state");
        assert!(state.is_empty());
        // Feed a few prices; result should be a MacdResult with
        // finite values (32 prices are needed for full MACD).
        let mut new_state = state_out;
        let mut result = EventBytes::EMPTY;
        for i in 0..32 {
            let price = (i as f64) * 1.0 + 100.0;
            let event_bytes = bincode::serialize(&price).unwrap();
            let rc = unsafe {
                ((*vtable).handle)(
                    new_state.ptr,
                    new_state.len,
                    event_bytes.as_ptr(),
                    event_bytes.len(),
                    &mut new_state,
                    &mut result,
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(rc, 0, "handle iter {i} returned {rc}");
        }
        let r_bytes = unsafe { std::slice::from_raw_parts(result.ptr, result.len) };
        let macd: MacdResult = bincode::deserialize(r_bytes).expect("decode result");
        assert!(macd.macd_line.is_finite());
        assert!(macd.signal_line.is_finite());
        assert!(macd.histogram.is_finite());
    }

    #[test]
    fn vtable_ema_init_state_and_handle() {
        let handle = TaLibFactory::init().expect("init");
        let vtable = *handle.handlers.get("EMA").expect("EMA vtable");
        let mut state_out = EventBytes::EMPTY;
        let rc = unsafe { ((*vtable).init_state)(&mut state_out) };
        assert_eq!(rc, 0);
        let prices: Vec<f64> = (1..=10).map(|x| x as f64).collect();
        let mut new_state = state_out;
        let mut result = EventBytes::EMPTY;
        for &p in &prices {
            let event_bytes = bincode::serialize(&p).unwrap();
            let rc = unsafe {
                ((*vtable).handle)(
                    new_state.ptr,
                    new_state.len,
                    event_bytes.as_ptr(),
                    event_bytes.len(),
                    &mut new_state,
                    &mut result,
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(rc, 0, "handle returned {rc}");
        }
        let r_bytes = unsafe { std::slice::from_raw_parts(result.ptr, result.len) };
        let ema_value: f64 = bincode::deserialize(r_bytes).expect("decode result");
        assert!(ema_value.is_finite());
        assert!(ema_value > 0.0, "ema_value={ema_value}");
    }

    #[test]
    fn vtable_decision_tree_handle_buy() {
        let handle = TaLibFactory::init().expect("init");
        let vtable = *handle
            .handlers
            .get("decision_tree")
            .expect("decision_tree vtable");
        let mut state = EventBytes::EMPTY;
        unsafe { ((*vtable).init_state)(&mut state) };
        let event_bytes = bincode::serialize(&(0.1_f64, 0.6_f64)).unwrap();
        let mut new_state = EventBytes::EMPTY;
        let mut result = EventBytes::EMPTY;
        let rc = unsafe {
            ((*vtable).handle)(
                state.ptr,
                state.len,
                event_bytes.as_ptr(),
                event_bytes.len(),
                &mut new_state,
                &mut result,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0);
        let r_bytes = unsafe { std::slice::from_raw_parts(result.ptr, result.len) };
        let decision: Decision = bincode::deserialize(r_bytes).expect("decode result");
        assert_eq!(decision, Decision::Buy);
    }

    #[test]
    fn vtable_decision_tree_handle_sell() {
        let handle = TaLibFactory::init().expect("init");
        let vtable = *handle
            .handlers
            .get("decision_tree")
            .expect("decision_tree vtable");
        let mut state = EventBytes::EMPTY;
        unsafe { ((*vtable).init_state)(&mut state) };
        let event_bytes = bincode::serialize(&(-0.1_f64, -0.6_f64)).unwrap();
        let mut new_state = EventBytes::EMPTY;
        let mut result = EventBytes::EMPTY;
        let rc = unsafe {
            ((*vtable).handle)(
                state.ptr,
                state.len,
                event_bytes.as_ptr(),
                event_bytes.len(),
                &mut new_state,
                &mut result,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0);
        let r_bytes = unsafe { std::slice::from_raw_parts(result.ptr, result.len) };
        let decision: Decision = bincode::deserialize(r_bytes).expect("decode result");
        assert_eq!(decision, Decision::Sell);
    }

    #[test]
    fn vtable_sentiment_analyzer_handle_positive() {
        let handle = TaLibFactory::init().expect("init");
        let vtable = *handle
            .handlers
            .get("sentiment_analyzer")
            .expect("sentiment_analyzer vtable");
        let mut state = EventBytes::EMPTY;
        unsafe { ((*vtable).init_state)(&mut state) };
        let event_bytes = bincode::serialize(&"bullish growth high buy rally adoption".to_string())
            .unwrap();
        let mut new_state = EventBytes::EMPTY;
        let mut result = EventBytes::EMPTY;
        let rc = unsafe {
            ((*vtable).handle)(
                state.ptr,
                state.len,
                event_bytes.as_ptr(),
                event_bytes.len(),
                &mut new_state,
                &mut result,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0);
        let r_bytes = unsafe { std::slice::from_raw_parts(result.ptr, result.len) };
        let score: f64 = bincode::deserialize(r_bytes).expect("decode result");
        assert!((score - 1.0).abs() < 1e-9, "score={score}");
    }

    #[test]
    fn vtable_handle_with_garbage_event_returns_error() {
        let handle = TaLibFactory::init().expect("init");
        let vtable = *handle.handlers.get("EMA").expect("EMA vtable");
        let mut state = EventBytes::EMPTY;
        unsafe { ((*vtable).init_state)(&mut state) };
        let garbage = vec![0xFFu8; 4];
        let mut new_state = EventBytes::EMPTY;
        let mut result = EventBytes::EMPTY;
        let rc = unsafe {
            ((*vtable).handle)(
                state.ptr,
                state.len,
                garbage.as_ptr(),
                garbage.len(),
                &mut new_state,
                &mut result,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, -1, "garbage event should return -1");
    }
}
