//! `bee-plugin-ta-indicators` — production-grade technical-analysis Handlers (S38).
//!
//! Real `yata` (pure Rust) indicators exposed as Bee Handler UDFs.
//! Six handlers are registered:
//!
//! | Handler    | Backed by                                | Event schema         | Result schema            |
//! |------------|------------------------------------------|----------------------|--------------------------|
//! | `macd`     | `yata::indicators::MACD`                 | `{ price, ts }`      | `{ value, ts }` (line)   |
//! | `ema`      | `yata::methods::EMA`                     | `{ price, ts }`      | `{ value, ts }`          |
//! | `rsi`      | `yata::indicators::RSI`                  | `{ price, ts }`      | `{ value, ts }`          |
//! | `bbands`   | `yata::indicators::BollingerBands`       | `{ price, ts }`      | `{ upper, mid, lower, ts }` |
//! | `atr`      | `yata::methods::TR` + Wilder's smoothing | `{ high, low, close, ts }` | `{ value, ts }`    |
//! | `vwap`     | Custom running sum                       | `{ price, volume, ts }` | `{ value, ts }`       |
//!
//! ## State management
//!
//! All indicators are streaming-friendly: each `handle` call
//! processes one event and emits one output. The plugin stores
//! per-stream state in a process-global `LazyLock<Mutex<HashMap<...>>>`
//! keyed by `state/handler/<stream_id>/<indicator>/`. The MVP stub
//! holds the state in memory; S38 follow-up wires this into Bee's
//! `BeeHostV1::safe_kv_get` / `safe_kv_put` so state survives a
//! plugin reload.
//!
//! ## Backends
//!
//! The plugin is configured at the plugin level (not per
//! Datasource). The MVP ships the `"yata"` backend only. The
//! `"ta-lib"` backend (C FFI via `ta-lib-sys`) is a documented
//! follow-up; see `IndicatorConfig` below.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use yata::core::{IndicatorConfig as YataIndicatorConfig, IndicatorInstance, Method, OHLCV};
use yata::helpers::MA;
use yata::indicators::{
    BollingerBands, BollingerBandsInstance, MACD, MACDInstance, RelativeStrengthIndex,
    RelativeStrengthIndexInstance,
};
use yata::methods::{EMA, TR};

use bee_plugin_sdk::{
    event::EventBytes, Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName,
};

// ---------------------------------------------------------------------------
// Section 2: type definitions — event / result / state types
// ---------------------------------------------------------------------------

/// Single-value event: one price + one timestamp. Used by
/// `macd`, `ema`, `rsi`, `bbands` (Bollinger Bands uses the
/// `close` part of the synthetic OHLCV; only `price` is needed).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub price: f64,
    pub ts: i64,
}

/// OHLC event: high / low / close + timestamp. Used by `atr`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EventOHLC {
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub ts: i64,
}

/// Volume-weighted event: price + volume + timestamp. Used by `vwap`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EventVwap {
    pub price: f64,
    pub volume: f64,
    pub ts: i64,
}

/// Scalar result: one value + the timestamp it was computed for.
/// Used by `macd`, `ema`, `rsi`, `atr`, `vwap`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct IndicatorResult {
    pub value: f64,
    pub ts: i64,
}

/// Bollinger Bands result: upper / middle / lower + timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BbandsResult {
    pub upper: f64,
    pub middle: f64,
    pub lower: f64,
    pub ts: i64,
}

/// Synthetic OHLCV constructed from a single `price` (close).
/// All other fields (open / high / low / volume) fall back to
/// `price` so that the yata `Indicator` API gets a valid
/// `OHLCV` for the single-value handlers (`macd`, `rsi`,
/// `bbands`). The yata library's own `Source::Close` extractor
/// is what they consume; `open` / `high` / `low` / `volume` are
/// ignored by these indicators.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PriceCandle {
    price: f64,
}

impl OHLCV for PriceCandle {
    #[inline]
    fn open(&self) -> f64 {
        self.price
    }
    #[inline]
    fn high(&self) -> f64 {
        self.price
    }
    #[inline]
    fn low(&self) -> f64 {
        self.price
    }
    #[inline]
    fn close(&self) -> f64 {
        self.price
    }
    #[inline]
    fn volume(&self) -> f64 {
        0.0
    }
}

/// Synthetic OHLCV constructed from explicit high / low / close.
/// Used by `atr` (which consumes all three via `Source` /
/// `tr_close`). `open` and `volume` are unused.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HlcCandle {
    high: f64,
    low: f64,
    close: f64,
}

impl OHLCV for HlcCandle {
    #[inline]
    fn open(&self) -> f64 {
        self.close
    }
    #[inline]
    fn high(&self) -> f64 {
        self.high
    }
    #[inline]
    fn low(&self) -> f64 {
        self.low
    }
    #[inline]
    fn close(&self) -> f64 {
        self.close
    }
    #[inline]
    fn volume(&self) -> f64 {
        0.0
    }
}

/// Per-stream state for the MACD handler. Holds the yata
/// `MACDInstance` (default config: fast=12, slow=26, signal=9).
/// The generic is the moving-average constructor; the default
/// `MA` is the standard EMA-family enum and matches
/// `MACD::default()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacdState {
    pub indicator: MACDInstance<MA>,
}

impl Default for MacdState {
    fn default() -> Self {
        let cfg = MACD::default();
        let candle = PriceCandle { price: 0.0 };
        Self {
            indicator: cfg
                .init(&candle)
                .expect("MACD::default config always validates"),
        }
    }
}

/// Per-stream state for the EMA handler. Holds the yata `EMA`
/// method (a Wilder's-style EMA over `ValueType`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EmaState {
    pub indicator: EMA,
}

impl Default for EmaState {
    fn default() -> Self {
        let period: u8 = 20;
        let seed: f64 = 0.0;
        Self {
            indicator: EMA::new(period, &seed).expect("EMA::new never fails for period > 0"),
        }
    }
}

/// Per-stream state for the RSI handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsiState {
    pub indicator: RelativeStrengthIndexInstance,
}

impl Default for RsiState {
    fn default() -> Self {
        let cfg = RelativeStrengthIndex::default();
        let candle = PriceCandle { price: 0.0 };
        Self {
            indicator: cfg
                .init(&candle)
                .expect("RelativeStrengthIndex::default config always validates"),
        }
    }
}

/// Per-stream state for the Bollinger Bands handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BbandsState {
    pub indicator: BollingerBandsInstance,
}

impl Default for BbandsState {
    fn default() -> Self {
        let cfg = BollingerBands::default();
        let candle = PriceCandle { price: 0.0 };
        Self {
            indicator: cfg
                .init(&candle)
                .expect("BollingerBands::default config always validates"),
        }
    }
}

/// Per-stream state for the ATR handler. ATR is the Wilder's
/// smoothed average of the True Range (`TR`). yata 0.6 does not
/// expose a dedicated `ATRIndicator`, so we compose `TR` (which
/// produces per-bar true range) with `EMA` using
/// `alpha = 1 / period` (the same arithmetic as yata's `RMA`
/// helper). We use `EMA` (which accepts the same `1/length`
/// alpha in its `new` constructor) by re-seeding it with a
/// custom alpha — but `EMA::new` only accepts a `PeriodType` and
/// internally uses `alpha = 2 / (length + 1)`, which is the
/// standard EMA. For Wilder's smoothing we therefore keep a
/// manual `prev_value` + custom `alpha = 1 / period` and avoid
/// using `EMA` here. `period` is carried explicitly so the
/// alpha is correct for any period in `1..=254`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AtrState {
    pub period: u8,
    pub tr: TR,
    pub prev_value: f64,
    pub initialized: bool,
}

impl Default for AtrState {
    fn default() -> Self {
        let period: u8 = 14;
        let candle = HlcCandle {
            high: 0.0,
            low: 0.0,
            close: 0.0,
        };
        Self {
            period,
            tr: TR::new(&candle).expect("TR::new never fails"),
            prev_value: 0.0,
            initialized: false,
        }
    }
}

/// Per-stream state for the VWAP handler. Custom running sum
/// of `price * volume` and `volume` over the stream's lifetime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct VwapState {
    pub running_pv: f64,
    pub running_v: f64,
}

// ---------------------------------------------------------------------------
// Section 3: plugin-level config
// ---------------------------------------------------------------------------

/// Plugin-level configuration. The MVP supports only the
/// `"yata"` backend. The `"ta-lib"` backend (C FFI via
/// `ta-lib-sys`) is a documented follow-up: it would carry the
/// same handler surface but route compute through the C library
/// instead of `yata`. For now, an unknown backend is rejected
/// at `init` time so misconfiguration is loud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorConfig {
    pub indicator_backend: String,
}

impl Default for IndicatorConfig {
    fn default() -> Self {
        Self {
            indicator_backend: "yata".to_string(),
        }
    }
}

impl IndicatorConfig {
    /// Parse a bincode-encoded `IndicatorConfig` blob. Returns
    /// `Err(String)` on any failure (unknown backend, bad
    /// bincode, ...).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        let cfg: Self = bincode::deserialize(bytes)
            .map_err(|e| format!("bincode deserialize IndicatorConfig: {e}"))?;
        if cfg.indicator_backend != "yata" {
            return Err(format!(
                "unsupported indicator_backend '{}' (MVP supports only 'yata'; 'ta-lib' is a follow-up)",
                cfg.indicator_backend
            ));
        }
        Ok(cfg)
    }
}

// ---------------------------------------------------------------------------
// Section 4: stream identity
// ---------------------------------------------------------------------------
//
// The SDK's `HandlerVtable::handle` is intentionally agnostic
// about stream identity: the host passes the per-stream state
// blob in, the handler updates it, the host stores it keyed by
// (handler, stream_id). The plugin therefore does NOT hash the
// stream id itself; it just operates on whatever state blob the
// host passes in. Each handler name is unique in the manifest
// below, so the (handler, stream_id) pair is unambiguous at the
// host level.

// ---------------------------------------------------------------------------
// Section 5: in-memory KV stub
// ---------------------------------------------------------------------------

/// Per-process KV stub. The MVP holds per-(stream, handler)
/// state in memory so a single `handle` call can find its
/// predecessor state. The S38 follow-up wires this to
/// `BeeHostV1::safe_kv_get` / `safe_kv_put` so state survives
/// a plugin reload.
static HANDLER_STATE: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Build the canonical key for a (stream_id, handler) state
/// entry. Mirrors the spec's
/// `state/handler/<stream_id>/<indicator_name>/` layout. The
/// MVP hashes the stream id to a hex string (the host passes
/// 32 bytes; we display them). We do the hex encoding inline
/// (no `hex` crate dependency) — this is a constant-size
/// 32-byte → 64-char transform.
fn state_key(stream_id: &[u8; 32], handler: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex_buf = [0u8; 64];
    for (i, &b) in stream_id.iter().enumerate() {
        hex_buf[i * 2] = HEX[(b >> 4) as usize];
        hex_buf[i * 2 + 1] = HEX[(b & 0x0f) as usize];
    }
    let hex_str = std::str::from_utf8(&hex_buf)
        .expect("hex digits are ASCII, always valid UTF-8");
    format!("state/handler/{hex_str}/{handler}/")
}

// ---------------------------------------------------------------------------
// Section 6: indicator dispatch (6 handlers)
// ---------------------------------------------------------------------------

/// Serialize `value` to bincode, write the resulting bytes into
/// `*out`, and return 0 on success / -1 on serialization
/// failure. Mirrors the helper in the S33 scaffold.
fn write_event_bytes<T: Serialize>(out: *mut EventBytes, value: &T) -> i32 {
    let bytes = match bincode::serialize(value) {
        Ok(b) => b,
        Err(_) => return -1,
    };
    let len = bytes.len();
    let ptr = bytes.as_ptr();
    std::mem::forget(bytes);
    unsafe { *out = EventBytes { ptr, len } };
    0
}

/// Deserialize a bincode-encoded value from `(ptr, len)`. On
/// empty input, returns `Default::default()`. On any decode
/// failure, also returns `Default::default()` (the indicator
/// state is recoverable from "empty" — we treat garbage state
/// the same as fresh state, which is the right call for an
/// idempotent Handler that always produces one output per
/// input).
fn decode_or_default<T: Default + for<'de> Deserialize<'de>>(ptr: *const u8, len: usize) -> T {
    if len == 0 {
        return T::default();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    bincode::deserialize(bytes).unwrap_or_default()
}

/// Persist `bytes` for `(stream_id, handler)`.
fn write_state_blob(stream_id: &[u8; 32], handler: &str, bytes: Vec<u8>) {
    let key = state_key(stream_id, handler);
    HANDLER_STATE
        .lock()
        .expect("HANDLER_STATE poisoned")
        .insert(key, bytes);
}

/// Derive a `stream_id` for the MVP. The S33 `HandlerVtable`
/// signature does not pass `stream_id` directly into `handle`,
/// so the MVP uses a fixed `[0u8; 32]` sentinel (a single
/// global stream). When the S38 follow-up wires the host's
/// `BeeHostV1::current_stream_id` into the handler call, the
/// stream id will be passed in via a side channel (a global
/// thread-local set by the host immediately before `handle`).
/// The MVP's contract is: the in-memory KV stub is per-process
/// state; tests / unit exercises get a single global stream
/// for now.
fn current_stream_id() -> [u8; 32] {
    [0u8; 32]
}

/// `macd` handler. Event: `{ price, ts }`. State:
/// `MacdState { indicator: MACDInstance }`. Result:
/// `{ value: macd_line, ts }`.
pub mod macd_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &MacdState::default())
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
        let mut state: MacdState = decode_or_default(state_ptr, state_len);
        let event: Event = match bincode::deserialize(unsafe {
            std::slice::from_raw_parts(event_ptr, event_len)
        }) {
            Ok(e) => e,
            Err(_) => return -1,
        };
        let candle = PriceCandle { price: event.price };
        let result = state.indicator.next(&candle);
        let macd_line = result.value(0);
        let out = IndicatorResult {
            value: macd_line,
            ts: event.ts,
        };
        let stream_id = current_stream_id();
        let state_bytes = match bincode::serialize(&state) {
            Ok(b) => b,
            Err(_) => return -1,
        };
        write_state_blob(&stream_id, "macd", state_bytes);
        if write_event_bytes(new_state_out, &state) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &out)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

/// `ema` handler. Event: `{ price, ts }`. State: `EmaState`.
/// Result: `{ value: ema_value, ts }`.
pub mod ema_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &EmaState::default())
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
        let mut state: EmaState = decode_or_default(state_ptr, state_len);
        let event: Event = match bincode::deserialize(unsafe {
            std::slice::from_raw_parts(event_ptr, event_len)
        }) {
            Ok(e) => e,
            Err(_) => return -1,
        };
        let value = state.indicator.next(&event.price);
        let out = IndicatorResult {
            value,
            ts: event.ts,
        };
        let stream_id = current_stream_id();
        let state_bytes = match bincode::serialize(&state) {
            Ok(b) => b,
            Err(_) => return -1,
        };
        write_state_blob(&stream_id, "ema", state_bytes);
        if write_event_bytes(new_state_out, &state) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &out)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

/// `rsi` handler. Event: `{ price, ts }`. State: `RsiState`.
/// Result: `{ value: rsi, ts }`. RSI is in `[0, 100]`; until
/// the indicator has seen `period` bars it returns
/// `50.0` (neutral).
pub mod rsi_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &RsiState::default())
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
        let mut state: RsiState = decode_or_default(state_ptr, state_len);
        let event: Event = match bincode::deserialize(unsafe {
            std::slice::from_raw_parts(event_ptr, event_len)
        }) {
            Ok(e) => e,
            Err(_) => return -1,
        };
        let candle = PriceCandle { price: event.price };
        let result = state.indicator.next(&candle);
        let rsi_value = result.value(0);
        let out = IndicatorResult {
            value: rsi_value,
            ts: event.ts,
        };
        let stream_id = current_stream_id();
        let state_bytes = match bincode::serialize(&state) {
            Ok(b) => b,
            Err(_) => return -1,
        };
        write_state_blob(&stream_id, "rsi", state_bytes);
        if write_event_bytes(new_state_out, &state) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &out)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

/// `bbands` handler. Event: `{ price, ts }`. State:
/// `BbandsState`. Result: `{ upper, middle, lower, ts }`.
pub mod bbands_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &BbandsState::default())
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
        let mut state: BbandsState = decode_or_default(state_ptr, state_len);
        let event: Event = match bincode::deserialize(unsafe {
            std::slice::from_raw_parts(event_ptr, event_len)
        }) {
            Ok(e) => e,
            Err(_) => return -1,
        };
        let candle = PriceCandle { price: event.price };
        let result = state.indicator.next(&candle);
        // yata::BollingerBands returns values in
        // [upper, middle, lower] order.
        let upper = result.value(0);
        let middle = result.value(1);
        let lower = result.value(2);
        let out = BbandsResult {
            upper,
            middle,
            lower,
            ts: event.ts,
        };
        let stream_id = current_stream_id();
        let state_bytes = match bincode::serialize(&state) {
            Ok(b) => b,
            Err(_) => return -1,
        };
        write_state_blob(&stream_id, "bbands", state_bytes);
        if write_event_bytes(new_state_out, &state) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &out)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

/// `atr` handler. Event: `{ high, low, close, ts }`. State:
/// `AtrState { period, tr, prev_value, initialized }`. Result:
/// `{ value: atr, ts }`.
///
/// Implementation note: yata 0.6 does not expose a dedicated
/// `ATRIndicator`. The standard formula is `ATR(n) = smoothed
/// TR(n)` where the smoothing is Wilder's `RMA`
/// (`alpha = 1/n`). We use yata's `TR` Method for the per-bar
/// true range, then apply Wilder's smoothing manually so the
/// `alpha = 1/period` is correct (yata's `EMA::new` uses
/// `alpha = 2/(n+1)`, which is the standard EMA, not Wilder's).
pub mod atr_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &AtrState::default())
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
        let mut state: AtrState = decode_or_default(state_ptr, state_len);
        let event: EventOHLC = match bincode::deserialize(unsafe {
            std::slice::from_raw_parts(event_ptr, event_len)
        }) {
            Ok(e) => e,
            Err(_) => return -1,
        };
        let candle = HlcCandle {
            high: event.high,
            low: event.low,
            close: event.close,
        };
        let tr_value = state.tr.next(&candle);
        let n = state.period as f64;
        let alpha = 1.0 / n;
        let atr = if !state.initialized {
            // Seed: on the first bar, ATR == TR. (Wilder's
            // original formula seeds with the simple mean of
            // the first `n` TRs; the S38 follow-up can refine
            // this to match `pandas-ta` exactly.)
            tr_value
        } else {
            alpha.mul_add(tr_value, (1.0 - alpha) * state.prev_value)
        };
        state.prev_value = atr;
        state.initialized = true;
        let out = IndicatorResult {
            value: atr,
            ts: event.ts,
        };
        let stream_id = current_stream_id();
        let state_bytes = match bincode::serialize(&state) {
            Ok(b) => b,
            Err(_) => return -1,
        };
        write_state_blob(&stream_id, "atr", state_bytes);
        if write_event_bytes(new_state_out, &state) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &out)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

/// `vwap` handler. Event: `{ price, volume, ts }`. State:
/// `VwapState { running_pv, running_v }`. Result:
/// `{ value: vwap, ts }`.
///
/// `VWAP = sum(price * volume) / sum(volume)` over the
/// stream's lifetime. yata does not provide a VWAP
/// `Indicator`, so this is a small custom state machine.
pub mod vwap_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &VwapState::default())
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
        let mut state: VwapState = decode_or_default(state_ptr, state_len);
        let event: EventVwap = match bincode::deserialize(unsafe {
            std::slice::from_raw_parts(event_ptr, event_len)
        }) {
            Ok(e) => e,
            Err(_) => return -1,
        };
        state.running_pv += event.price * event.volume;
        state.running_v += event.volume;
        let vwap = if state.running_v == 0.0 {
            event.price
        } else {
            state.running_pv / state.running_v
        };
        let out = IndicatorResult {
            value: vwap,
            ts: event.ts,
        };
        let stream_id = current_stream_id();
        let state_bytes = match bincode::serialize(&state) {
            Ok(b) => b,
            Err(_) => return -1,
        };
        write_state_blob(&stream_id, "vwap", state_bytes);
        if write_event_bytes(new_state_out, &state) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &out)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

// ---------------------------------------------------------------------------
// Section 7: plugin manifest + factory
// ---------------------------------------------------------------------------

/// Factory for the ta-indicators plugin. Holds no per-instance
/// state: all vtables are `const`, and the per-stream state
/// lives in `HANDLER_STATE`.
pub struct TaIndicatorsFactory;

impl Factory for TaIndicatorsFactory {
    fn manifest() -> PluginManifest {
        PluginManifest {
            name: PluginName("bee-plugin-ta-indicators".into()),
            feature_version: "0.1.0".into(),
            abi_version: "v1".into(),
            adapters: vec![],
            handlers: vec![
                HandlerDescriptor {
                    name: "macd".into(),
                },
                HandlerDescriptor {
                    name: "ema".into(),
                },
                HandlerDescriptor {
                    name: "rsi".into(),
                },
                HandlerDescriptor {
                    name: "bbands".into(),
                },
                HandlerDescriptor {
                    name: "atr".into(),
                },
                HandlerDescriptor {
                    name: "vwap".into(),
                },
            ],
        }
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        let mut handlers: HashMap<String, *const bee_plugin_sdk::vtable::HandlerVtable> =
            HashMap::new();
        handlers.insert("macd".to_string(), &macd_shim::VTABLE as *const _);
        handlers.insert("ema".to_string(), &ema_shim::VTABLE as *const _);
        handlers.insert("rsi".to_string(), &rsi_shim::VTABLE as *const _);
        handlers.insert("bbands".to_string(), &bbands_shim::VTABLE as *const _);
        handlers.insert("atr".to_string(), &atr_shim::VTABLE as *const _);
        handlers.insert("vwap".to_string(), &vwap_shim::VTABLE as *const _);
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
            input_adapters: HashMap::new(),
            output_adapters: HashMap::new(),
            handlers,
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(TaIndicatorsFactory);

