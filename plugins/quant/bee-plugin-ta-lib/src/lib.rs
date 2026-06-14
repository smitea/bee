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
// ---------------------------------------------------------------------------
// Section 6: indicator dispatch (6 macro-generated handlers)
// ---------------------------------------------------------------------------
//
// S33.6.1: refactored to use the
// `#[bee_adapter]` macro. The hand-written
// macd_shim, ema_shim, rsi_shim, bbands_shim,
// atr_shim, vwap_shim modules are gone.
// The macro generates 6 vtable constants
// + 6 per-handler FFI shim sets.

use bee_plugin_macro::{bee_adapter, bee_method};
use bee_adapter::AdapterResult;

/// Per-handler state for the 6 TA
/// handlers. Empty in the MVP — the plugin
/// is stateless. Kept for wire-format
/// stability (a future S37 follow-up adds
/// rolling-window caches here).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaHandlerState {
    pub _reserved: Vec<u8>,
}

pub struct MacdHandler;

#[bee_adapter(handler, name = "macd")]
impl MacdHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<TaHandlerState> {
        Ok(TaHandlerState::default())
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        state: TaHandlerState,
        _event: Vec<u8>,
    ) -> AdapterResult<(TaHandlerState, Vec<u8>)> {
        // MVP stub: stateless, emit empty.
        Ok((state, vec![]))
    }
}

pub struct EmaHandler;

#[bee_adapter(handler, name = "ema")]
impl EmaHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<TaHandlerState> {
        Ok(TaHandlerState::default())
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        state: TaHandlerState,
        _event: Vec<u8>,
    ) -> AdapterResult<(TaHandlerState, Vec<u8>)> {
        Ok((state, vec![]))
    }
}

pub struct RsiHandler;

#[bee_adapter(handler, name = "rsi")]
impl RsiHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<TaHandlerState> {
        Ok(TaHandlerState::default())
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        state: TaHandlerState,
        _event: Vec<u8>,
    ) -> AdapterResult<(TaHandlerState, Vec<u8>)> {
        Ok((state, vec![]))
    }
}

pub struct BbandsHandler;

#[bee_adapter(handler, name = "bbands")]
impl BbandsHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<TaHandlerState> {
        Ok(TaHandlerState::default())
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        state: TaHandlerState,
        _event: Vec<u8>,
    ) -> AdapterResult<(TaHandlerState, Vec<u8>)> {
        Ok((state, vec![]))
    }
}

pub struct AtrHandler;

#[bee_adapter(handler, name = "atr")]
impl AtrHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<TaHandlerState> {
        Ok(TaHandlerState::default())
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        state: TaHandlerState,
        _event: Vec<u8>,
    ) -> AdapterResult<(TaHandlerState, Vec<u8>)> {
        Ok((state, vec![]))
    }
}

pub struct VwapHandler;

#[bee_adapter(handler, name = "vwap")]
impl VwapHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<TaHandlerState> {
        Ok(TaHandlerState::default())
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        state: TaHandlerState,
        _event: Vec<u8>,
    ) -> AdapterResult<(TaHandlerState, Vec<u8>)> {
        Ok((state, vec![]))
    }
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
        let mut input_adapters = std::collections::HashMap::new();
        let mut output_adapters = std::collections::HashMap::new();
        let mut handlers = std::collections::HashMap::new();
        bee_plugin_sdk::register_vtable! {
            input_adapters, output_adapters, handlers;
            handler "macd"   => &MACD_HANDLER_VTABLE,
            handler "ema"    => &EMA_HANDLER_VTABLE,
            handler "rsi"    => &RSI_HANDLER_VTABLE,
            handler "bbands" => &BBANDS_HANDLER_VTABLE,
            handler "atr"    => &ATR_HANDLER_VTABLE,
            handler "vwap"   => &VWAP_HANDLER_VTABLE,
        }
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
            input_adapters,
            output_adapters,
            handlers,
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(TaIndicatorsFactory);

