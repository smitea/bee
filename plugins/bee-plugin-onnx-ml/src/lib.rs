//! `bee-plugin-onnx-ml` — production-grade ONNX ML model Handlers (S39).
//!
//! Session 1 (S39 b): **SKELETON ONLY** — config, plugin manifest, 4 dummy
//! Handler functions. Real `tract-onnx` runtime + FinBERT model loading
//! + tokenization + batching will be added in a follow-up session.
//!
//! Per [S39 in the quant stories](../../../../docs/best-practices/quant/stories.md#s39--bee-plugin-onnx-ml-production-grade-onnx-ml-model-handlers-real-tract-runtime--finbert).
//!
//! ## Handler contract (4 SQL UDFs)
//!
//! | Handler             | Signature                              | Dummy result                                |
//! |---------------------|----------------------------------------|---------------------------------------------|
//! | `sentiment_score`   | `sentiment_score(text_col)`            | `SentimentResult { score: 0.0, class: Neutral }` |
//! | `sentiment_class`   | `sentiment_class(text_col)`            | `SentimentClass::Neutral`                   |
//! | `price_direction`   | `price_direction(features_struct)`     | `DirectionResult { direction: Flat }`       |
//! | `model_score`       | `model_score(model_name, features_struct)` | `ModelScoreResult { score: 0.0 }`       |
//!
//! ## What the future sessions add
//!
//! - `tract-onnx::prelude::SimplePlan<f32, TypedModel>` loaded from
//!   `OnnxConfig::sentiment_model_path` and `decision_model_path`.
//! - `tokenizers::Tokenizer::from_file(...)` for FinBERT WordPiece.
//! - Batching: the plugin accumulates up to `max_batch_size` calls and
//!   flushes them as a single `tract` inference (transparent to SQL).
//! - Real `score` values: FinBERT logits → softmax → `[-1, 1]` map.
//! - Real `direction` classification from the user-supplied decision model.
//!
//! ## Constraints honored by this session
//!
//! - **No `unsafe` outside the FFI shim bodies** (the 4 vtable shims).
//! - **No network calls; no model file loading yet** (the skeleton uses
//!   hardcoded dummy values).
//! - **No real tract calls** (those come in a follow-up session).

#![allow(unused_imports, dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use bee_plugin_sdk::{
    event::EventBytes, Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName,
};

// ---------------------------------------------------------------------------
// Section 2: type definitions — event / result / state types
// ---------------------------------------------------------------------------

/// Three-way sentiment classification. The future FinBERT integration
/// will map softmax probabilities to one of these three classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SentimentClass {
    Positive,
    Neutral,
    Negative,
}

impl SentimentClass {
    /// Stable lowercase string form (the spec mandates
    /// `{"positive", "neutral", "negative"}`).
    pub fn as_str(&self) -> &'static str {
        match self {
            SentimentClass::Positive => "positive",
            SentimentClass::Neutral => "neutral",
            SentimentClass::Negative => "negative",
        }
    }
}

/// Three-way price direction classification. The future decision-model
/// integration will map the model's argmax to one of these three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Up,
    Down,
    Flat,
}

impl Direction {
    /// Stable lowercase string form (the spec mandates
    /// `{"up", "down", "flat"}`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
            Direction::Flat => "flat",
        }
    }
}

/// Result type for `sentiment_score` and `sentiment_class`. The
/// `score` is the contract value (in `[-1, 1]`); `class` is the
/// derived three-way label. The two are computed together so the
/// SQL user can pick whichever shape is convenient.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SentimentResult {
    pub score: f32,
    pub class: SentimentClass,
}

/// Result type for `price_direction`. The `direction` is the model's
/// argmax over `{"up", "down", "flat"}`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DirectionResult {
    pub direction: Direction,
}

/// Result type for `model_score`. The `score` is the model's raw
/// output (interpreted downstream by the SQL user).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelScoreResult {
    pub score: f32,
}

/// Event shape for `sentiment_score` / `sentiment_class`. A piece of
/// text (news headline, article body, social post) plus the source
/// timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEvent {
    pub text: String,
    pub ts: i64,
}

/// Event shape for `price_direction`. A bag of feature floats
/// (the spec calls this `features_struct`; on the FFI wire it's
/// bincode-encoded). The exact field set is decided by the SQL
/// caller via `struct_pack(...)` in the Pipeline; the plugin does
/// not interpret individual fields — it just hands the whole
/// payload to the decision model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeaturesEvent {
    pub features: Vec<f32>,
    pub ts: i64,
}

/// Event shape for `model_score`. The SQL caller picks the model by
/// name (e.g. `"sentiment"`, `"decision"`, or any user-supplied
/// model the plugin was configured to load). The future session
/// will look up the model in the `ModelRegistry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelScoreEvent {
    pub model_name: String,
    pub features: Vec<f32>,
    pub ts: i64,
}

// ---------------------------------------------------------------------------
// Section 3: plugin-level config
// ---------------------------------------------------------------------------

/// Plugin-level configuration for `bee-plugin-onnx-ml`. Mirrors the
/// S39 spec exactly:
///
/// ```jsonc
/// {
///   "sentiment_model_path": "./models/finbert-quant.onnx",
///   "decision_model_path":  "./models/btc-direction-1h.onnx",
///   "max_batch_size":       32,
///   "device":               "cpu"
/// }
/// ```
///
/// The S39 (b) skeleton does NOT load the files — it only parses
/// and stores the config. The follow-up session will thread this
/// into `load_models`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnnxConfig {
    pub sentiment_model_path: String,
    pub decision_model_path: String,
    pub max_batch_size: usize,
    /// `"cpu"` for MVP. `"gpu"` is a 1.x follow-up; the skeleton
    /// accepts any string and the future session will reject
    /// anything other than `"cpu"`.
    pub device: String,
}

impl Default for OnnxConfig {
    fn default() -> Self {
        Self {
            sentiment_model_path: "./models/finbert-quant.onnx".to_string(),
            decision_model_path: "./models/btc-direction-1h.onnx".to_string(),
            max_batch_size: 32,
            device: "cpu".to_string(),
        }
    }
}

impl OnnxConfig {
    /// Parse a plugin config blob. Matches the S38 `IndicatorConfig`
    /// pattern: an empty blob means "use the default"; a non-empty
    /// blob is bincode-deserialized.
    ///
    /// Note on wire format: the S39 spec *describes* the config in
    /// JSON form (for documentation / human readers). The on-the-
    /// wire format on the FFI boundary is bincode, mirroring how
    /// `bee-plugin-ta-indicators` carries `IndicatorConfig`. This
    /// keeps the plugin's direct dependencies minimal (no
    /// `serde_json` in `Cargo.toml`); the future host tooling can
    /// transcode the JSON form to bincode at registration time.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        let cfg: Self = bincode::deserialize(bytes)
            .map_err(|e| format!("bincode deserialize OnnxConfig: {e}"))?;
        // The MVP is CPU-only; the future session will widen this
        // to support "gpu" and validate the field. For now any
        // non-empty `device` string is accepted; the runtime
        // doesn't read it.
        Ok(cfg)
    }
}

// ---------------------------------------------------------------------------
// Section 4: model loading stub
// ---------------------------------------------------------------------------

/// Placeholder for a loaded ONNX session.
///
/// The follow-up session will replace this with
/// `pub type ModelHandle = tract_onnx::prelude::SimplePlan<f32, tract_onnx::prelude::TypedModel>;`
/// and `load_models` will read the `.onnx` files and compile them
/// into a `SimplePlan`. The skeleton uses `()` so the
/// `ModelRegistry` shape is already correct and the swap is a
/// type-only change.
pub type ModelHandle = ();

/// Holds (in the future) the loaded ONNX sessions keyed by logical
/// model name. The skeleton's `ModelRegistry` is intentionally
/// empty — `load_models` returns `Ok` without touching disk. The
/// follow-up session will:
///
/// 1. `tract_onnx::onnx()` the sentiment model file → `SimplePlan`.
/// 2. `tract_onnx::onnx()` the decision model file → `SimplePlan`.
/// 3. Store both in `ModelRegistry` under their logical names.
/// 4. Build a `tokenizers::Tokenizer` for FinBERT's WordPiece.
pub struct ModelRegistry {
    /// The FinBERT sentiment model. `None` in the skeleton; the
    /// follow-up session will populate it.
    pub sentiment_model: Option<ModelHandle>,
    /// The user-supplied decision model. `None` in the skeleton;
    /// the follow-up session will populate it.
    pub decision_model: Option<ModelHandle>,
    /// The parsed `OnnxConfig`, kept for batching-size lookups
    /// during inference.
    pub config: OnnxConfig,
}

impl ModelRegistry {
    /// Stub: return an empty `ModelRegistry` without loading any
    /// model files. The follow-up session will replace the body
    /// with real `tract-onnx` loading.
    pub fn empty(config: OnnxConfig) -> Self {
        Self {
            sentiment_model: None,
            decision_model: None,
            config,
        }
    }
}

/// Stub model loader. In the skeleton this returns an empty
/// `ModelRegistry` without touching the filesystem. The follow-up
/// session will:
/// 1. Verify both model files exist on disk.
/// 2. Parse + compile each into a `SimplePlan`.
/// 3. Return a populated `ModelRegistry`.
///
/// `#[allow(dead_code)]` keeps the function reachable for the
/// follow-up session even though no caller in the skeleton invokes
/// it.
#[allow(dead_code)]
pub fn load_models(_config: &OnnxConfig) -> Result<ModelRegistry, OnnxError> {
    Ok(ModelRegistry::empty(_config.clone()))
}

/// Error type for the future `load_models`. The skeleton has no
/// constructors yet, but the type is in place so the
/// `ModelRegistry` API surface doesn't churn.
#[derive(Debug, thiserror::Error)]
pub enum OnnxError {
    /// The follow-up session will add:
    /// - `ModelFileNotFound { path: String }`
    /// - `ModelParseError { path: String, source: tract_onnx::TractError }`
    /// - `UnsupportedDevice(String)`
    /// - `TokenizerError(tokenizers::Error)`
    #[error("onnx load error: {0}")]
    Stub(String),
}

// ---------------------------------------------------------------------------
// Section 5: dummy handler implementations
// ---------------------------------------------------------------------------
//
// Each of the 4 handlers exposes a typed `xxx_dummy(&[u8]) -> XxxResult`
// function. The dummy ignores the input bytes (the decoder is still
// there so the FFI shim's type system is satisfied) and returns a
// hardcoded placeholder. The follow-up session will replace the body
// of each with: decode → tokenize → batch → run model → postprocess.

/// Dummy `sentiment_score` implementation. Returns a neutral
/// placeholder (`score: 0.0, class: Neutral`). The follow-up
/// session will:
///
/// 1. Decode the `TextEvent`.
/// 2. Tokenize with FinBERT's WordPiece tokenizer.
/// 3. Run inference via the sentiment `SimplePlan`.
/// 4. softmax → argmax → `SentimentClass`; logits → `[-1, 1]` map.
pub fn sentiment_score_dummy(_event_bytes: &[u8]) -> SentimentResult {
    SentimentResult {
        score: 0.0,
        class: SentimentClass::Neutral,
    }
}

/// Dummy `sentiment_class` implementation. Returns `Neutral`. The
/// follow-up session will derive this from FinBERT's argmax.
pub fn sentiment_class_dummy(_event_bytes: &[u8]) -> SentimentClass {
    SentimentClass::Neutral
}

/// Dummy `price_direction` implementation. Returns `Flat`. The
/// follow-up session will derive this from the decision model's
/// argmax over the three direction classes.
pub fn price_direction_dummy(_event_bytes: &[u8]) -> DirectionResult {
    DirectionResult {
        direction: Direction::Flat,
    }
}

/// Dummy `model_score` implementation. Returns `0.0`. The
/// follow-up session will run the named model and return its raw
/// output (float or class index).
pub fn model_score_dummy(_event_bytes: &[u8]) -> ModelScoreResult {
    ModelScoreResult { score: 0.0 }
}

// ---------------------------------------------------------------------------
// Section 6: FFI vtables (4 vtables)
// ---------------------------------------------------------------------------
//
// Each handler's vtable has:
// - `init_state`: write a default `OnnxHandlerState` (empty for
//   now; the future session may add batching buffers).
// - `handle`: decode the event bytes (we tolerate garbage by
//   returning the dummy result), call the typed dummy function,
//   write the result into `*result_out`, write a copy of the
//   (unchanged) state into `*new_state_out`, and return 0.
//
// Memory ownership: the FFI shims `forget` the `Vec<u8>` so the
// pointer + length remain valid for the host to read. The host
// frees the bytes via the standard `bee-plugin-sdk` allocator
// path. This matches the S38 `ta-indicators` pattern exactly.

/// Per-stream state for the 4 ONNX handlers. In the skeleton
/// this is empty — Handlers have no inherent state; the future
/// session may add a small batching buffer per (stream, handler)
/// so a burst of `sentiment_score` calls can be coalesced into a
/// single `tract` inference.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnnxHandlerState {
    /// Reserved for the future batching buffer. The skeleton
    /// emits an empty state; the follow-up session will add
    /// `pending: Vec<TextEvent>` and a "flushed?" flag.
    pub _reserved: Vec<u8>,
}

/// Serialize `value` to bincode, write the resulting bytes into
/// `*out`, and return 0 on success / -1 on serialization failure.
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

/// `sentiment_score` handler. Event: `TextEvent { text, ts }`.
/// State: `OnnxHandlerState`. Result: `SentimentResult`.
pub mod sentiment_score_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &OnnxHandlerState::default())
    }

    pub unsafe extern "C" fn handle(
        _state_ptr: *const u8,
        _state_len: usize,
        _event_ptr: *const u8,
        _event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        // The skeleton ignores the event bytes entirely and
        // returns a hardcoded neutral result. The follow-up
        // session will:
        //   1. bincode-deserialize a `TextEvent` (tolerating
        //      garbage by falling back to the dummy).
        //   2. Look up the sentiment model in `ModelRegistry`.
        //   3. Tokenize + run inference (batched up to
        //      `max_batch_size`).
        //   4. Map logits → `SentimentResult`.
        let result = sentiment_score_dummy(&[]);
        if write_event_bytes(new_state_out, &OnnxHandlerState::default()) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &result)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

/// `sentiment_class` handler. Event: `TextEvent { text, ts }`.
/// State: `OnnxHandlerState`. Result: `SentimentClass`.
pub mod sentiment_class_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &OnnxHandlerState::default())
    }

    pub unsafe extern "C" fn handle(
        _state_ptr: *const u8,
        _state_len: usize,
        _event_ptr: *const u8,
        _event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        let result = sentiment_class_dummy(&[]);
        if write_event_bytes(new_state_out, &OnnxHandlerState::default()) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &result)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

/// `price_direction` handler. Event: `FeaturesEvent { features, ts }`.
/// State: `OnnxHandlerState`. Result: `DirectionResult`.
pub mod price_direction_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &OnnxHandlerState::default())
    }

    pub unsafe extern "C" fn handle(
        _state_ptr: *const u8,
        _state_len: usize,
        _event_ptr: *const u8,
        _event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        let result = price_direction_dummy(&[]);
        if write_event_bytes(new_state_out, &OnnxHandlerState::default()) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &result)
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

/// `model_score` handler. Event:
/// `ModelScoreEvent { model_name, features, ts }`. State:
/// `OnnxHandlerState`. Result: `ModelScoreResult`.
pub mod model_score_shim {
    use super::*;

    pub unsafe extern "C" fn init_state(out: *mut EventBytes) -> i32 {
        write_event_bytes(out, &OnnxHandlerState::default())
    }

    pub unsafe extern "C" fn handle(
        _state_ptr: *const u8,
        _state_len: usize,
        _event_ptr: *const u8,
        _event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        _err_out: *mut EventBytes,
    ) -> i32 {
        let result = model_score_dummy(&[]);
        if write_event_bytes(new_state_out, &OnnxHandlerState::default()) != 0 {
            return -1;
        }
        write_event_bytes(result_out, &result)
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

/// Factory for the onnx-ml plugin. Holds no per-instance state:
/// all vtables are `const`, the `ModelRegistry` lives in plugin
/// memory (populated by the future `load_models` call), and the
/// per-stream state is the empty `OnnxHandlerState` for now.
pub struct OnnxMlFactory;

impl Factory for OnnxMlFactory {
    fn manifest() -> PluginManifest {
        PluginManifest {
            name: PluginName("bee-plugin-onnx-ml".into()),
            feature_version: "0.1.0".into(),
            abi_version: "v1".into(),
            // No Adapters: this is a Handler-only plugin. The
            // S39 spec calls out: "this is a **Handler** plugin.
            // No Datasource config; the plugin registers SQL UDFs
            // that wrap ONNX models loaded from disk."
            adapters: vec![],
            handlers: vec![
                HandlerDescriptor {
                    name: "sentiment_score".into(),
                },
                HandlerDescriptor {
                    name: "sentiment_class".into(),
                },
                HandlerDescriptor {
                    name: "price_direction".into(),
                },
                HandlerDescriptor {
                    name: "model_score".into(),
                },
            ],
        }
    }

    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        // The follow-up session will:
        //   1. Read the plugin-level config from the host.
        //   2. Call `load_models(&cfg)` to build the
        //      `ModelRegistry`.
        //   3. Stash the `Arc<ModelRegistry>` in
        //      `inner: Arc::new(registry)`.
        //
        // For the skeleton, `inner` is `Arc::new(())` and the
        // `ModelRegistry` is not built.
        let mut handlers: HashMap<String, *const bee_plugin_sdk::vtable::HandlerVtable> =
            HashMap::new();
        handlers.insert(
            "sentiment_score".to_string(),
            &sentiment_score_shim::VTABLE as *const _,
        );
        handlers.insert(
            "sentiment_class".to_string(),
            &sentiment_class_shim::VTABLE as *const _,
        );
        handlers.insert(
            "price_direction".to_string(),
            &price_direction_shim::VTABLE as *const _,
        );
        handlers.insert(
            "model_score".to_string(),
            &model_score_shim::VTABLE as *const _,
        );
        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: Arc::new(()),
            input_adapters: HashMap::new(),
            output_adapters: HashMap::new(),
            handlers,
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(OnnxMlFactory);
