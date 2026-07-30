//! `bee_plugin_sample_kline` — sample Bee plugin used to verify the
//! "Reload from disk" flow in Bee Client's Settings > Plugins.
//!
//! The plugin exposes:
//!
//! - an Input Adapter `kline` whose open() decodes a bincode-encoded
//!   `KlineConfig { url, symbol, interval }`. `next()` emits one
//!   synthetic price event so the surrounding UI has something to
//!   inspect without requiring a live upstream.
//! - a Handler `ema` whose state is an `EmaState { alpha, prev }` and
//!   whose event is an `f64` price. `handle` produces the new EMA
//!   value: `alpha * price + (1 - alpha) * prev_ema` (or just
//!   `price` for the first event).
//! - an Output Adapter `emit` that records every emitted event in an
//!   in-memory `Vec<EmaResult>` (the host's `close` drops it).
//!
//! The plugin deliberately stays self-contained — no real network
//! calls, no host KV — so the Reload from disk flow can be exercised
//! without any external dependencies.

use std::sync::Arc;

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::bee_adapter;
use bee_plugin_sdk::{
    AdapterDescriptor, Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName,
    PluginResult,
};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct KlineConfig {
    pub url: String,
    pub symbol: String,
    pub interval: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct KlineEvent {
    pub symbol: String,
    pub price: f64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct EmaResult {
    pub symbol: String,
    pub ema: f64,
}

pub struct KlineInput;

#[bee_adapter(input, name = "kline")]
impl KlineInput {
    #[bee_method(slot = "open")]
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let _cfg: KlineConfig = bincode::deserialize(&config)
            .map_err(|e| AdapterError::Open(format!("kline config: {e}")))?;
        Ok(Self)
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        let payload = bincode::serialize(&KlineEvent {
            symbol: "BTC/USDT".to_string(),
            price: 100.0,
        })
        .map_err(|e| AdapterError::Next(format!("encode: {e}")))?;
        Ok(Some(Event {
            timestamp: bee_adapter::Event::now_timestamp(),
            sequence: 0,
            payload,
        }))
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}

pub struct EmaHandler;

#[bee_adapter(handler, name = "ema")]
impl EmaHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<EmaState> {
        Ok(EmaState {
            alpha: 0.2,
            prev: None,
        })
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        mut state: EmaState,
        event: EmaEvent,
    ) -> AdapterResult<(EmaState, EmaResult)> {
        let prev_ema = state.prev.unwrap_or(event.price);
        let new_ema = state.alpha.mul_add(event.price, (1.0 - state.alpha) * prev_ema);
        state.prev = Some(new_ema);
        Ok((
            state,
            EmaResult {
                symbol: event.symbol,
                ema: new_ema,
            },
        ))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct EmaState {
    pub alpha: f64,
    pub prev: Option<f64>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct EmaEvent {
    pub symbol: String,
    pub price: f64,
}

pub struct EmitOutput {
    rows: Arc<std::sync::Mutex<Vec<EmaResult>>>,
}

#[bee_adapter(output, name = "emit")]
impl EmitOutput {
    #[bee_method(slot = "open")]
    pub async fn open(_config: Vec<u8>) -> AdapterResult<Self> {
        Ok(Self {
            rows: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    #[bee_method(slot = "emit")]
    pub async fn emit_one(&mut self, event: Event) -> AdapterResult<()> {
        let row: EmaResult = bincode::deserialize(&event.payload)
            .map_err(|e| AdapterError::Emit(format!("decode: {e}")))?;
        self.rows.lock().map_err(|e| AdapterError::Emit(e.to_string()))?.push(row);
        Ok(())
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}

pub fn plugin_manifest() -> PluginManifest {
    PluginManifest {
        name: PluginName("sample-kline".into()),
        feature_version: "0.1.0".into(),
        abi_version: "v1".into(),
        adapters: vec![
            AdapterDescriptor {
                name: "kline".into(),
                is_input: true,
            },
            AdapterDescriptor {
                name: "emit".into(),
                is_input: false,
            },
        ],
        handlers: vec![HandlerDescriptor {
            name: "ema".into(),
        }],
    }
}

pub struct SampleKlineFactory;

impl Factory for SampleKlineFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> PluginResult<PluginHandle> {
        let mut input_adapters = std::collections::HashMap::new();
        let mut output_adapters = std::collections::HashMap::new();
        let mut handlers = std::collections::HashMap::new();

        bee_plugin_sdk::register_vtable! {
            input_adapters, output_adapters, handlers;
            input "kline" => &KLINE_INPUT_VTABLE,
            output "emit" => &EMIT_OUTPUT_VTABLE,
            handler "ema" => &EMA_HANDLER_VTABLE,
        }

        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: Arc::new(()),
            input_adapters,
            output_adapters,
            handlers,
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(SampleKlineFactory);