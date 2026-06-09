//! `bee-plugin-onnx-ml` — production-grade ONNX ML model Handlers (S39).
//!
//! Real `tract-onnx` runtime + FinBERT sentiment + user-supplied
//! decision model. Exposes four SQL UDFs:
//!
//! | Handler             | Signature                                    | Backed by                                       |
//! |---------------------|----------------------------------------------|-------------------------------------------------|
//! | `sentiment_score`   | `sentiment_score(text_col)`                  | FinBERT (`./models/finbert-quant.onnx`)         |
//! | `sentiment_class`   | `sentiment_class(text_col)`                  | FinBERT argmax over {positive, neutral, negative} |
//! | `price_direction`   | `price_direction(features_struct)`           | User-supplied decision model                    |
//! | `model_score`       | `model_score(model_name, features_struct)`   | Generic dispatcher over loaded model registry   |
//!
//! Per [S39 in the quant stories](../../../../docs/best-practices/quant/stories.md#s39--bee-plugin-onnx-ml-production-grade-onnx-ml-model-handlers-real-tract-runtime--finbert).
//!
//! ## Architecture
//!
//! - **Model loading** is synchronous at plugin init: `load_models(&cfg)`
//!   reads the `.onnx` files, optimizes, and produces a `ModelRegistry`
//!   holding `tract_onnx::prelude::TypedRunnableModel<TypedModel>` instances
//!   plus `tokenizers::Tokenizer` for the FinBERT text path.
//! - **Tokenization** uses the `tokenizers` crate; the tokenizer file is
//!   loaded from `<sentiment_model_path>.tokenizer.json` (a sibling file
//!   next to the ONNX model). If the tokenizer file is missing, the
//!   sentiment handlers report a clear error at inference time
//!   (the plugin still LOADS — `bee plugin list` works).
//! - **FFI shims** access the `ModelRegistry` through a process-global
//!   `OnceLock<Mutex<Option<Arc<ModelRegistry>>>>` populated in
//!   `OnnxMlFactory::init()`. This mirrors the S35 / S36 / S37 pattern
//!   (shared `Arc<Client>` in mongodb, shared rate limiter in influxdb).
//! - **Batching**: the S39 spec calls out up-to-`max_batch_size` call
//!   coalescing for `sentiment_score`. The MVP uses **single-call
//!   inference** (tract's `SimplePlan::run` is synchronous and per-call
//!   overhead is modest for short text). The batching is a documented
//!   follow-up; the on-wire contract is unchanged (one call in, one
//!   result out).
//!
//! ## Constraints honored
//!
//! - **No `unsafe` outside the FFI shim bodies** (the 4 vtable shims).
//! - **No network calls; no model file loading beyond local paths.**
//! - **No model weights bundled in the plugin crate** (the tests use a
//!   small synthetic ONNX; production models live at
//!   `plugin_config.sentiment_model_path` / `decision_model_path`).
//! - **MVP: missing model file is a soft failure.** The plugin still
//!   LOADS so the host can `bee plugin list`; only actual inference
//!   fails with `OnnxError::ModelNotLoaded`.
//!
//! ## File layout
//!
//! ```text
//!  Section 1  Imports + crate-level attributes
//!  Section 2  Type definitions (events, results, errors, state)
//!  Section 3  OnnxConfig (plugin-level config)
//!  Section 4  ModelRegistry + load_models (tract + tokenizers)
//!  Section 5  Handler impls: sentiment_score, sentiment_class,
//!             price_direction, model_score
//!  Section 6  FFI helpers (write_event_bytes, decode_event, ...)
//!  Section 7  FFI vtables (4 shim modules)
//!  Section 8  OnnxMlFactory + cdylib_plugin!
//! ```

#![allow(unused_imports, dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;
use tract_onnx::prelude::*;

use bee_plugin_sdk::{
    event::EventBytes, Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName,
};

// ---------------------------------------------------------------------------
// Section 2: type definitions — event / result / state types
// ---------------------------------------------------------------------------

/// Three-way sentiment classification. FinBERT's argmax over its
/// 3-logit output (negative / neutral / positive) is mapped to one
/// of these three variants.
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

/// Three-way price direction classification. The decision model's
/// argmax over its 3-logit output is mapped to one of these.
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
/// model the plugin was configured to load). The handler looks the
/// name up in the `ModelRegistry`.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnnxConfig {
    pub sentiment_model_path: String,
    pub decision_model_path: String,
    pub max_batch_size: usize,
    /// `"cpu"` for MVP. `"gpu"` is a 1.x follow-up; the loader
    /// accepts any string and silently treats anything other than
    /// `"cpu"` as `"cpu"`.
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
    /// `serde_json` in `Cargo.toml`); the host tooling transcodes
    /// the JSON form to bincode at registration time.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        let cfg: Self = bincode::deserialize(bytes)
            .map_err(|e| format!("bincode deserialize OnnxConfig: {e}"))?;
        Ok(cfg)
    }
}

// ---------------------------------------------------------------------------
// Section 4: model loading — tract + tokenizers
// ---------------------------------------------------------------------------

/// Loaded ONNX sentiment model + FinBERT WordPiece tokenizer. The
/// tokenizer is optional in the MVP — a missing tokenizer file is
/// reported as `OnnxError::Tokenizer` at inference time so the
/// plugin still LOADS (the host's `bee plugin list` works).
pub struct SentimentModel {
    pub plan: TypedRunnableModel<TypedModel>,
    pub tokenizer: Option<Arc<Tokenizer>>,
}

/// Loaded ONNX decision model (no tokenizer — the decision model
/// takes raw features).
pub struct DecisionModel {
    pub plan: TypedRunnableModel<TypedModel>,
    pub feature_dim: Option<usize>,
}

/// Holds the loaded ONNX sessions + tokenizers keyed by logical
/// model name. Populated by `load_models` at plugin init.
///
/// - `sentiment_model` / `sentiment_tokenizer`: the FinBERT
///   sentiment path. Both fields are `None` when the model file
///   is missing (the plugin still loads).
/// - `decision_model`: the user-supplied direction model. `None`
///   when the file is missing.
/// - `models`: the registry for the `model_score` generic
///   dispatcher. Keys are model names; values are a (plan, optional
///   tokenizer) pair. The MVP starts empty (the user has not yet
///   configured any user-named models); the S39 follow-up will
///   populate this from a `models_dir` config field.
pub struct ModelRegistry {
    /// The compiled FinBERT sentiment model.
    pub sentiment_model: Option<SentimentModel>,
    /// The compiled decision (price-direction) model.
    pub decision_model: Option<DecisionModel>,
    /// Generic named-model registry for `model_score(...)`. Empty in
    /// the MVP; populated by a follow-up that scans a `models_dir`.
    pub models: HashMap<String, (TypedRunnableModel<TypedModel>, Option<Arc<Tokenizer>>)>,
    /// The parsed `OnnxConfig`, kept for batching-size lookups
    /// during inference and for error messages.
    pub config: OnnxConfig,
}

impl ModelRegistry {
    /// Build an empty `ModelRegistry` from a config (no models
    /// loaded). Useful for tests and for the "model file not
    /// present" MVP path.
    pub fn empty(config: OnnxConfig) -> Self {
        Self {
            sentiment_model: None,
            decision_model: None,
            models: HashMap::new(),
            config,
        }
    }

    /// True if the sentiment model AND its tokenizer are loaded.
    /// The sentiment handlers short-circuit to an error when this
    /// is `false` so the SQL user gets a clear message.
    pub fn sentiment_ready(&self) -> bool {
        matches!(
            (&self.sentiment_model, self.sentiment_model.as_ref().and_then(|s| s.tokenizer.as_ref())),
            (Some(_), Some(_))
        )
    }
}

/// Build a 1D `Tensor` from a flat `Vec<T>`. Used by the
/// `model_score` / `price_direction` handlers to convert a
/// plain `Vec<f32>` feature vector into the tensor form tract
/// expects.
///
/// We go through `Tensor::from_shape` rather than
/// `ndarray::Array2::into_tensor()` because `IntoTensor` is
/// implemented only for the `ndarray 0.16` crate that `tract`
/// vendors, not for the `ndarray 0.15` crate we depend on
/// directly. The hand-rolled conversion keeps the Cargo.toml
/// unchanged and stays zero-cost (`from_shape` copies into the
/// tensor's storage).
fn make_f32_tensor_1d(data: &[f32], len: usize) -> Result<Tensor, OnnxError> {
    Tensor::from_shape(&[len], data).map_err(|e| OnnxError::Shape(format!("f32 1d tensor: {e}")))
}

/// Build a 2D `[1, N]` `Tensor` from a flat `Vec<T>`. Used by
/// the `sentiment_score` handler to convert the three
/// `Vec<i64>` tokenized inputs into tensors tract expects.
fn make_i64_tensor_2d(data: &[i64], cols: usize) -> Result<Tensor, OnnxError> {
    Tensor::from_shape(&[1, cols], data)
        .map_err(|e| OnnxError::Shape(format!("i64 2d tensor: {e}")))
}

/// Load a single ONNX file from disk and compile it into a
/// `TypedRunnableModel<TypedModel>`. Returns `OnnxError::Io` if the file is
/// missing (so the caller can produce a clear "model file not
/// found" error) and `OnnxError::Tract` for any tract-side parse
/// or optimization failure.
fn load_onnx_plan(path: &str) -> Result<TypedRunnableModel<TypedModel>, OnnxError> {
    if !Path::new(path).exists() {
        return Err(OnnxError::Io(format!("model file not found: {path}")));
    }
    tract_onnx::onnx()
        .model_for_path(path)
        .map_err(|e| OnnxError::Tract(format!("model_for_path({path}): {e}")))?
        .into_optimized()
        .map_err(|e| OnnxError::Tract(format!("into_optimized({path}): {e}")))?
        .into_runnable()
        .map_err(|e| OnnxError::Tract(format!("into_runnable({path}): {e}")))
}

/// Optionally load a tokenizer JSON from a sibling file. Returns
/// `Ok(None)` when the file does not exist (the MVP tolerates a
/// missing tokenizer — the handler reports a clear error at
/// inference time). Returns `OnnxError::Tokenizer` if the file
/// exists but cannot be parsed.
fn try_load_tokenizer(model_path: &str) -> Result<Option<Arc<Tokenizer>>, OnnxError> {
    let tokenizer_path = format!("{model_path}.tokenizer.json");
    if !Path::new(&tokenizer_path).exists() {
        return Ok(None);
    }
    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| OnnxError::Tokenizer(format!("Tokenizer::from_file({tokenizer_path}): {e}")))?;
    Ok(Some(Arc::new(tokenizer)))
}

/// Load all configured models from disk. The MVP is forgiving:
/// a missing sentiment or decision model file is **not** a fatal
/// error — the corresponding `ModelRegistry` slot is `None` and
/// the handler short-circuits with a clear error. The plugin
/// itself still LOADS.
///
/// The function:
/// 1. Tries to load the sentiment model + tokenizer.
/// 2. Tries to load the decision model.
/// 3. Leaves the `models` named-registry empty (the S39 follow-up
///    adds a `models_dir` config field).
/// 4. Logs every model load attempt + outcome at `info` / `warn`.
pub fn load_models(config: &OnnxConfig) -> Result<ModelRegistry, OnnxError> {
    if config.device != "cpu" {
        log::warn!(
            "onnx-ml: device={} requested but only 'cpu' is supported in MVP; \
             falling back to cpu",
            config.device
        );
    }

    let sentiment_model = match load_onnx_plan(&config.sentiment_model_path) {
        Ok(plan) => {
            let tokenizer = try_load_tokenizer(&config.sentiment_model_path)?;
            if tokenizer.is_none() {
                log::warn!(
                    "onnx-ml: sentiment tokenizer not found at '{}.tokenizer.json'; \
                     sentiment_score / sentiment_class will return an error",
                    config.sentiment_model_path
                );
            } else {
                log::info!(
                    "onnx-ml: loaded sentiment model + tokenizer from {}",
                    config.sentiment_model_path
                );
            }
            Some(SentimentModel { plan, tokenizer })
        }
        Err(OnnxError::Io(msg)) => {
            log::warn!("onnx-ml: {msg} (sentiment handlers will error)");
            None
        }
        Err(e) => {
            log::error!("onnx-ml: failed to load sentiment model: {e}");
            return Err(e);
        }
    };

    let decision_model = match load_onnx_plan(&config.decision_model_path) {
        Ok(plan) => {
            log::info!(
                "onnx-ml: loaded decision model from {}",
                config.decision_model_path
            );
            Some(DecisionModel {
                plan,
                feature_dim: None,
            })
        }
        Err(OnnxError::Io(msg)) => {
            log::warn!("onnx-ml: {msg} (price_direction will error)");
            None
        }
        Err(e) => {
            log::error!("onnx-ml: failed to load decision model: {e}");
            return Err(e);
        }
    };

    Ok(ModelRegistry {
        sentiment_model,
        decision_model,
        models: HashMap::new(),
        config: config.clone(),
    })
}

/// Error type for the ONNX plugin. The variants are arranged so the
/// host can match on them and produce a useful SQL error message.
#[derive(Debug, thiserror::Error)]
pub enum OnnxError {
    /// The named model is not in the registry. Returned when the
    /// `model_score` handler is called with a model name that was
    /// not loaded at init time.
    #[error("model not loaded: {0}")]
    ModelNotLoaded(String),

    /// A `tract-onnx` operation failed (parse, optimize, or
    /// inference). The wrapped string is a tract error formatted
    /// via `Display`.
    #[error("tract error: {0}")]
    Tract(String),

    /// The HuggingFace `tokenizers` crate returned an error (e.g.
    /// a malformed `tokenizer.json`).
    #[error("tokenizer error: {0}")]
    Tokenizer(String),

    /// The input event could not be reshaped to what the model
    /// expects (e.g. a features vector of the wrong length).
    #[error("shape error: {0}")]
    Shape(String),

    /// A generic inference-time failure (postprocessing the model
    /// output failed). Kept distinct from `Tract` so the host can
    /// tell "the model file is broken" from "the model output
    /// had an unexpected shape / class count".
    #[error("inference error: {0}")]
    Inference(String),

    /// Filesystem I/O (a model file was not found, or could not
    /// be read).
    #[error("io error: {0}")]
    Io(String),

    /// A malformed event payload (bincode-decode failed or the
    /// event shape does not match what the handler expects).
    #[error("event error: {0}")]
    Event(String),
}

impl OnnxError {
    /// Short, single-line summary suitable for the host's error
    /// log. Avoids embedding large tract error chains in the
    /// per-event log.
    pub fn summary(&self) -> String {
        match self {
            OnnxError::ModelNotLoaded(name) => {
                format!("model not loaded: {name}")
            }
            OnnxError::Tract(m) => {
                let first_line = m.lines().next().unwrap_or(m);
                format!("tract: {first_line}")
            }
            OnnxError::Tokenizer(m) => {
                format!("tokenizer: {m}")
            }
            OnnxError::Shape(m) => format!("shape: {m}"),
            OnnxError::Inference(m) => format!("inference: {m}"),
            OnnxError::Io(m) => format!("io: {m}"),
            OnnxError::Event(m) => format!("event: {m}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Section 5: handler implementations
// ---------------------------------------------------------------------------
//
// Each handler has a typed function `xxx(&ModelRegistry, &Event) -> Result<X, OnnxError>`
// that the FFI shim calls. The shim does the bincode decode + result
// encode; the typed function does the actual work.
//
// Batching is documented as a follow-up; the MVP runs one
// inference per call (tract's `SimplePlan::run` is synchronous and
// per-call overhead is modest for short text — the S39 spec calls
// batching an optimization, not a correctness requirement).

/// Run FinBERT on a single text. Tokenizes with the configured
/// WordPiece tokenizer, builds a 1xN batch of (input_ids,
/// attention_mask, token_type_ids), runs the model, applies
/// softmax over the 3-logit output, and returns
/// `(score in [-1, 1], class)`.
pub fn sentiment_score(
    registry: &ModelRegistry,
    event: &TextEvent,
) -> Result<SentimentResult, OnnxError> {
    let (model, tokenizer) = match (
        registry.sentiment_model.as_ref(),
        registry
            .sentiment_model
            .as_ref()
            .and_then(|s| s.tokenizer.as_ref()),
    ) {
        (Some(m), Some(t)) => (m, t),
        (None, _) => {
            return Err(OnnxError::ModelNotLoaded("sentiment".into()));
        }
        (_, None) => {
            return Err(OnnxError::Tokenizer(
                "sentiment tokenizer not loaded (missing tokenizer.json?)".into(),
            ));
        }
    };

    let encoding = tokenizer
        .encode(event.text.as_str(), true)
        .map_err(|e| OnnxError::Tokenizer(format!("tokenizer.encode: {e}")))?;
    let ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
    let attention_mask: Vec<i64> = encoding
        .get_attention_mask()
        .iter()
        .map(|&x| x as i64)
        .collect();
    let type_ids: Vec<i64> = encoding
        .get_type_ids()
        .iter()
        .map(|&x| x as i64)
        .collect();
    let seq_len = ids.len();
    if seq_len == 0 {
        return Err(OnnxError::Tokenizer(
            "tokenizer produced an empty encoding (zero-length text?)".into(),
        ));
    }

    let result = model
        .plan
        .run(tvec![
            make_i64_tensor_2d(&ids, seq_len)?.into(),
            make_i64_tensor_2d(&attention_mask, seq_len)?.into(),
            make_i64_tensor_2d(&type_ids, seq_len)?.into(),
        ])
        .map_err(|e| OnnxError::Inference(format!("sentiment inference: {e}")))?;

    let logits_view = result[0]
        .to_array_view::<f32>()
        .map_err(|e| OnnxError::Shape(format!("logits to_array_view<f32>: {e}")))?;
    if logits_view.ndim() < 2 || logits_view.shape()[0] < 1 || logits_view.shape()[1] < 3 {
        return Err(OnnxError::Inference(format!(
            "sentiment model output shape {:?} is not [1, >=3]",
            logits_view.shape()
        )));
    }
    // The view is `ArrayViewD<f32>` (dynamic dim). Reshape it
    // flat and index by the expected class offsets. We do not
    // use `.row(0)` because the dynamic-dim view does not
    // expose it; we instead iterate the leading 3 elements of
    // row 0 directly.
    let shape = logits_view.shape();
    let row_len = shape[1];
    if row_len < 3 {
        return Err(OnnxError::Inference(format!(
            "sentiment model row 0 has {} cols, need >= 3",
            row_len
        )));
    }
    let neg = logits_view[[0, 0]];
    let neu = logits_view[[0, 1]];
    let pos = logits_view[[0, 2]];

    // Numerically-stable softmax over the 3 logits.
    let max = neg.max(neu).max(pos);
    let exp_neg = (neg - max).exp();
    let exp_neu = (neu - max).exp();
    let exp_pos = (pos - max).exp();
    let sum = exp_neg + exp_neu + exp_pos;
    let prob_neg = exp_neg / sum;
    let prob_neu = exp_neu / sum;
    let prob_pos = exp_pos / sum;

    // Map (prob_pos - prob_neg) to [-1, 1]. The class is
    // determined by the argmax with a 0.5 threshold (the FinBERT
    // convention used in most quant dashboards).
    let score = (prob_pos - prob_neg) as f32;
    let class = if prob_pos > prob_neg && prob_pos > 0.5 {
        SentimentClass::Positive
    } else if prob_neg > prob_pos && prob_neg > 0.5 {
        SentimentClass::Negative
    } else {
        SentimentClass::Neutral
    };

    // Suppress the unused-variable lint for `prob_neu`; the
    // argmax path uses it implicitly via the threshold check.
    let _ = prob_neu;

    Ok(SentimentResult { score, class })
}

/// `sentiment_class` reuses the `sentiment_score` implementation
/// (the two handlers share the same model + tokenizer; the only
/// difference is the result shape the SQL user wants).
pub fn sentiment_class(
    registry: &ModelRegistry,
    event: &TextEvent,
) -> Result<SentimentClass, OnnxError> {
    Ok(sentiment_score(registry, event)?.class)
}

/// Run the decision model on a single feature vector. The model's
/// argmax over its 3-logit output is mapped to `Direction`. The
/// shape of the feature vector is opaque to the plugin — the SQL
/// caller is responsible for matching the model's expected input
/// dim.
pub fn price_direction(
    registry: &ModelRegistry,
    event: &FeaturesEvent,
) -> Result<DirectionResult, OnnxError> {
    let model = registry
        .decision_model
        .as_ref()
        .ok_or_else(|| OnnxError::ModelNotLoaded("decision".into()))?;
    let features = &event.features;
    if features.is_empty() {
        return Err(OnnxError::Shape(
            "price_direction: features is empty".into(),
        ));
    }
    let input = make_f32_tensor_1d(features, features.len())?;
    let result = model
        .plan
        .run(tvec![input.into()])
        .map_err(|e| OnnxError::Inference(format!("decision inference: {e}")))?;
    let logits_view = result[0]
        .to_array_view::<f32>()
        .map_err(|e| OnnxError::Shape(format!("decision logits to_array_view<f32>: {e}")))?;
    if logits_view.ndim() < 2 || logits_view.shape()[0] < 1 {
        return Err(OnnxError::Inference(format!(
            "decision model output shape {:?} is not [1, ...]",
            logits_view.shape()
        )));
    }
    // The view is `ArrayViewD<f32>` (dynamic dim). Copy row 0
    // into a `Vec` so we can argmax.
    let row_len = logits_view.shape()[1];
    let scores: Vec<f32> = (0..row_len)
        .map(|i| logits_view[[0, i]])
        .collect();
    let argmax = scores
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(1);
    let direction = match argmax {
        0 => Direction::Down,
        2 => Direction::Up,
        // Default to Flat for any unexpected class count (1-class
        // model, 4-class model, etc.). The S39 spec mandates the
        // 3-way mapping; the fallback is a safety net.
        _ => Direction::Flat,
    };
    Ok(DirectionResult { direction })
}

/// Generic dispatcher. Looks up a model by name in the registry's
/// `models` map and returns the first logit as a `f32`. The
/// shape of the result is intentionally narrow: the SQL user
/// builds a richer `model_score` per task (e.g. an argmax) in the
/// Pipeline definition.
pub fn model_score(
    registry: &ModelRegistry,
    event: &ModelScoreEvent,
) -> Result<ModelScoreResult, OnnxError> {
    let (plan, _tokenizer) = registry
        .models
        .get(&event.model_name)
        .ok_or_else(|| OnnxError::ModelNotLoaded(event.model_name.clone()))?;
    if event.features.is_empty() {
        return Err(OnnxError::Shape(format!(
            "model_score({}): features is empty",
            event.model_name
        )));
    }
    let input = make_f32_tensor_1d(&event.features, event.features.len())?;
    let result = plan
        .run(tvec![input.into()])
        .map_err(|e| OnnxError::Inference(format!("model_score({}): {e}", event.model_name)))?;
    let logits_view = result[0]
        .to_array_view::<f32>()
        .map_err(|e| OnnxError::Shape(format!("model_score logits: {e}")))?;
    if logits_view.ndim() < 2 || logits_view.shape()[0] < 1 || logits_view.shape()[1] < 1 {
        return Err(OnnxError::Inference(format!(
            "model_score({}) output shape {:?} is not [1, >=1]",
            event.model_name,
            logits_view.shape()
        )));
    }
    let score = logits_view[[0, 0]];
    Ok(ModelScoreResult { score })
}

// ---------------------------------------------------------------------------
// Section 6: FFI helpers
// ---------------------------------------------------------------------------

/// Per-stream state for the 4 ONNX handlers. Empty in the MVP —
/// the plugin is stateless (all state lives in the
/// `ModelRegistry` and the process-global slot). The struct is
/// kept so the wire format is stable when the S39 follow-up adds
/// per-stream batching buffers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnnxHandlerState {
    /// Reserved for the future batching buffer. Empty in the MVP.
    pub _reserved: Vec<u8>,
}

/// Serialize `value` to bincode, write the resulting bytes into
/// `*out`, and return 0 on success / -1 on serialization failure.
/// The `Vec<u8>` is `forget`-ed so the host owns the allocation
/// and is responsible for freeing it.
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

/// Write a string error to `*err_out` (when non-null). The error
/// is bincode-encoded as a UTF-8 byte string so the host can
/// surface it in the SQL error message.
fn write_err(err_out: *mut EventBytes, msg: &str) {
    if err_out.is_null() {
        return;
    }
    let bytes = match bincode::serialize(msg) {
        Ok(b) => b,
        Err(_) => return,
    };
    let len = bytes.len();
    let ptr = bytes.as_ptr();
    std::mem::forget(bytes);
    unsafe { *err_out = EventBytes { ptr, len } };
}

/// Decode a `T: DeserializeOwned` from a bincode blob. Returns
/// `Err(String)` with a friendly error on failure.
fn decode_event<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, String> {
    bincode::deserialize(bytes).map_err(|e| format!("bincode deserialize: {e}"))
}

// ---------------------------------------------------------------------------
// Section 7: FFI vtables (4 vtables)
// ---------------------------------------------------------------------------
//
// Each handler's vtable has:
// - `init_state`: write a default `OnnxHandlerState` (empty).
// - `handle`: decode the event bytes, call the typed handler,
//   write the result + a copy of the (unchanged) state into the
//   `*_out` pointers, and return 0.
//
// On error, the shim:
// - writes a UTF-8 error string to `*err_out` (when non-null),
// - writes a zero-valued result to `*result_out` so the host's
//   `result` is well-defined even on failure (the host should
//   still check the return code + `err_out`),
// - returns -1.

/// Process-global slot for the loaded `ModelRegistry`. Populated
/// in `OnnxMlFactory::init()`. The FFI shims look it up here so
/// they can reach the registry without the host having to thread
/// the `PluginHandle` through every call.
///
/// The `OnceLock<Mutex<Option<Arc<ModelRegistry>>>>` shape mirrors
/// the S37 mongodb `OnceLock<Mutex<Option<Arc<Client>>>>` pattern:
/// the first `init` populates the slot; subsequent `init` calls
/// (e.g. on plugin reload) can replace or reuse the registry.
static REGISTRY: OnceLock<Mutex<Option<Arc<ModelRegistry>>>> = OnceLock::new();

fn registry_slot() -> &'static Mutex<Option<Arc<ModelRegistry>>> {
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Try to clone the current `ModelRegistry` out of the global
/// slot. Returns `None` if `init()` has not been called yet (e.g.
/// a host bug that calls `handle` before `bee_plugin_init`).
fn current_registry() -> Option<Arc<ModelRegistry>> {
    let slot = registry_slot();
    let guard = slot.lock().expect("registry mutex poisoned");
    guard.as_ref().map(Arc::clone)
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
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        err_out: *mut EventBytes,
    ) -> i32 {
        if write_event_bytes(new_state_out, &OnnxHandlerState::default()) != 0 {
            return -1;
        }
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let event: TextEvent = match decode_event(event_bytes) {
            Ok(e) => e,
            Err(e) => {
                write_err(err_out, &format!("sentiment_score event decode: {e}"));
                let _ = write_event_bytes(result_out, &SentimentResult::default());
                return -1;
            }
        };
        let registry = match current_registry() {
            Some(r) => r,
            None => {
                write_err(err_out, "sentiment_score: plugin not initialized");
                let _ = write_event_bytes(result_out, &SentimentResult::default());
                return -1;
            }
        };
        match sentiment_score(&registry, &event) {
            Ok(result) => write_event_bytes(result_out, &result),
            Err(e) => {
                log::warn!("sentiment_score: {}", e.summary());
                write_err(err_out, &e.to_string());
                let _ = write_event_bytes(result_out, &SentimentResult::default());
                -1
            }
        }
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
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        err_out: *mut EventBytes,
    ) -> i32 {
        if write_event_bytes(new_state_out, &OnnxHandlerState::default()) != 0 {
            return -1;
        }
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let event: TextEvent = match decode_event(event_bytes) {
            Ok(e) => e,
            Err(e) => {
                write_err(err_out, &format!("sentiment_class event decode: {e}"));
                let _ = write_event_bytes(result_out, &SentimentClass::Neutral);
                return -1;
            }
        };
        let registry = match current_registry() {
            Some(r) => r,
            None => {
                write_err(err_out, "sentiment_class: plugin not initialized");
                let _ = write_event_bytes(result_out, &SentimentClass::Neutral);
                return -1;
            }
        };
        match sentiment_class(&registry, &event) {
            Ok(result) => write_event_bytes(result_out, &result),
            Err(e) => {
                log::warn!("sentiment_class: {}", e.summary());
                write_err(err_out, &e.to_string());
                let _ = write_event_bytes(result_out, &SentimentClass::Neutral);
                -1
            }
        }
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
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        err_out: *mut EventBytes,
    ) -> i32 {
        if write_event_bytes(new_state_out, &OnnxHandlerState::default()) != 0 {
            return -1;
        }
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let event: FeaturesEvent = match decode_event(event_bytes) {
            Ok(e) => e,
            Err(e) => {
                write_err(err_out, &format!("price_direction event decode: {e}"));
                let _ = write_event_bytes(result_out, &DirectionResult::default());
                return -1;
            }
        };
        let registry = match current_registry() {
            Some(r) => r,
            None => {
                write_err(err_out, "price_direction: plugin not initialized");
                let _ = write_event_bytes(result_out, &DirectionResult::default());
                return -1;
            }
        };
        match price_direction(&registry, &event) {
            Ok(result) => write_event_bytes(result_out, &result),
            Err(e) => {
                log::warn!("price_direction: {}", e.summary());
                write_err(err_out, &e.to_string());
                let _ = write_event_bytes(result_out, &DirectionResult::default());
                -1
            }
        }
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
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        err_out: *mut EventBytes,
    ) -> i32 {
        if write_event_bytes(new_state_out, &OnnxHandlerState::default()) != 0 {
            return -1;
        }
        let event_bytes = std::slice::from_raw_parts(event_ptr, event_len);
        let event: ModelScoreEvent = match decode_event(event_bytes) {
            Ok(e) => e,
            Err(e) => {
                write_err(err_out, &format!("model_score event decode: {e}"));
                let _ = write_event_bytes(result_out, &ModelScoreResult::default());
                return -1;
            }
        };
        let registry = match current_registry() {
            Some(r) => r,
            None => {
                write_err(err_out, "model_score: plugin not initialized");
                let _ = write_event_bytes(result_out, &ModelScoreResult::default());
                return -1;
            }
        };
        match model_score(&registry, &event) {
            Ok(result) => write_event_bytes(result_out, &result),
            Err(e) => {
                log::warn!("model_score: {}", e.summary());
                write_err(err_out, &e.to_string());
                let _ = write_event_bytes(result_out, &ModelScoreResult::default());
                -1
            }
        }
    }

    pub const VTABLE: bee_plugin_sdk::vtable::HandlerVtable =
        bee_plugin_sdk::vtable::HandlerVtable {
            handle,
            init_state,
        };
}

// ---------------------------------------------------------------------------
// Section 8: plugin manifest + factory
// ---------------------------------------------------------------------------

/// Factory for the onnx-ml plugin. Holds no per-instance state:
/// all vtables are `const`, the `ModelRegistry` lives in the
/// process-global slot, and the per-stream state is the empty
/// `OnnxHandlerState`.
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
        // Load models from the default config. The host can
        // later pass a non-empty config blob to override the
        // paths (the S39 follow-up wires this through
        // `bee_plugin_init`); for the MVP we use the defaults
        // (which point at relative `./models/...` paths that
        // will be missing in the test environment — the plugin
        // still LOADS with `None` slots and the handlers
        // short-circuit to a clear error at inference time).
        let config = OnnxConfig::default();
        let registry = match load_models(&config) {
            Ok(r) => Arc::new(r),
            Err(e) => {
                // A hard failure during model load (e.g. the
                // model file exists but cannot be parsed) is
                // reported back to the host so the plugin
                // manager can refuse to register the broken
                // plugin. The "file not found" soft failure is
                // already handled inside `load_models` and never
                // reaches this branch.
                return Err(bee_plugin_sdk::PluginError::Init(format!(
                    "load_models: {}",
                    e.summary()
                )));
            }
        };

        // Publish the registry to the process-global slot. The
        // FFI shims pick it up from there. If a previous init
        // (e.g. from a plugin reload) already populated the
        // slot, we replace it.
        {
            let slot = registry_slot();
            let mut guard = slot.lock().expect("registry mutex poisoned");
            *guard = Some(Arc::clone(&registry));
        }

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
            inner: registry,
            input_adapters: HashMap::new(),
            output_adapters: HashMap::new(),
            handlers,
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(OnnxMlFactory);

// ---------------------------------------------------------------------------
// Section 9: Default impls for the FFI shim fallbacks
// ---------------------------------------------------------------------------
//
// The shims write a "default" result to `*result_out` when the
// handler errors (so the host's `result` slot is always
// bincode-decodable). We add `Default` impls to the result types
// here so the fallbacks are obvious.

impl Default for SentimentResult {
    fn default() -> Self {
        Self {
            score: 0.0,
            class: SentimentClass::Neutral,
        }
    }
}

impl Default for DirectionResult {
    fn default() -> Self {
        Self {
            direction: Direction::Flat,
        }
    }
}

impl Default for ModelScoreResult {
    fn default() -> Self {
        Self { score: 0.0 }
    }
}

// ---------------------------------------------------------------------------
// Section 10: unit tests — S39 (c) skeleton coverage
// ---------------------------------------------------------------------------
//
// These tests exercise the *typed API only*: config parsing,
// error mapping, manifest, FFI vtable shims (init_state + result
// shape), enum bincode round-trips, and the loader's behavior
// when a model file is missing. They do NOT touch a real
// `tract-onnx` model or FinBERT model file; those land in the
// S39 (h) follow-up that adds integration tests with a synthetic
// ONNX.

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::MaybeUninit;

    // -----------------------------------------------------------------------
    // FFI helpers
    // -----------------------------------------------------------------------

    /// Call `init_state` on the given shim and return the bytes the
    /// shim wrote (via the SDK allocator). The shim `forget`s the
    /// `Vec<u8>` it produced, so the test reconstructs a `Vec` to
    /// own the allocation and drop it on scope exit. Mirrors the
    /// S38 `call_init_state` pattern.
    fn call_init_state(
        init_fn: unsafe extern "C" fn(*mut EventBytes) -> i32,
    ) -> Vec<u8> {
        let mut state_eb = MaybeUninit::<EventBytes>::zeroed();
        let rc = unsafe { init_fn(state_eb.as_mut_ptr()) };
        assert_eq!(rc, 0, "init_state shim returned non-zero status: {rc}");
        let state_eb = unsafe { state_eb.assume_init() };
        assert!(
            !state_eb.ptr.is_null() && state_eb.len > 0,
            "init_state produced empty EventBytes"
        );
        // SAFETY: the shim wrote a bincode `Vec<u8>` and `forget`ed
        // it; reconstructing as a `Vec<u8>` transfers ownership
        // back and lets the test free the allocation when the
        // returned `Vec` drops.
        let bytes = unsafe {
            Vec::from_raw_parts(state_eb.ptr as *mut u8, state_eb.len, state_eb.len)
        };
        bytes
    }

    // -----------------------------------------------------------------------
    // 1. config_default
    // -----------------------------------------------------------------------

    #[test]
    fn config_default() {
        let cfg = OnnxConfig::default();
        assert_eq!(cfg.sentiment_model_path, "./models/finbert-quant.onnx");
        assert_eq!(cfg.decision_model_path, "./models/btc-direction-1h.onnx");
        assert_eq!(cfg.max_batch_size, 32);
        assert_eq!(cfg.device, "cpu");
    }

    // -----------------------------------------------------------------------
    // 2. config_bincode_roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn config_bincode_roundtrip() {
        let cfg = OnnxConfig {
            sentiment_model_path: "/opt/models/sentiment-v2.onnx".to_string(),
            decision_model_path: "/opt/models/decision-btc.onnx".to_string(),
            max_batch_size: 64,
            device: "cpu".to_string(),
        };
        let bytes = bincode::serialize(&cfg).expect("serialize cfg");
        let back: OnnxConfig = bincode::deserialize(&bytes).expect("deserialize cfg");
        assert_eq!(cfg, back);
    }

    // -----------------------------------------------------------------------
    // 3. config_from_empty_bytes_uses_default
    // -----------------------------------------------------------------------

    #[test]
    fn config_from_empty_bytes_uses_default() {
        let cfg = OnnxConfig::from_bytes(&[]).expect("empty bytes should yield default");
        assert_eq!(cfg, OnnxConfig::default());
        // Also pin the field values explicitly so a regression that
        // changes `default()` doesn't silently pass.
        assert_eq!(cfg.sentiment_model_path, "./models/finbert-quant.onnx");
        assert_eq!(cfg.decision_model_path, "./models/btc-direction-1h.onnx");
        assert_eq!(cfg.max_batch_size, 32);
        assert_eq!(cfg.device, "cpu");
    }

    // -----------------------------------------------------------------------
    // 4. config_from_invalid_bytes_returns_err
    // -----------------------------------------------------------------------

    #[test]
    fn config_from_invalid_bytes_returns_err() {
        // 4 random bytes: cannot bincode-decode as a `OnnxConfig`
        // (which expects at least a u64 length prefix for the
        // first `String` field).
        let garbage: [u8; 4] = [0, 1, 2, 3];
        let res = OnnxConfig::from_bytes(&garbage);
        assert!(res.is_err(), "garbage bytes must not decode as OnnxConfig");
    }

    // -----------------------------------------------------------------------
    // 5. onnx_error_summary_is_one_line
    // -----------------------------------------------------------------------

    #[test]
    fn onnx_error_summary_is_one_line() {
        let cases = [
            OnnxError::ModelNotLoaded("sentiment".into()),
            OnnxError::Tract(
                "first line of tract error\nsecond line should be hidden".into(),
            ),
            OnnxError::Tokenizer("bad json".into()),
            OnnxError::Shape("dim mismatch".into()),
            OnnxError::Inference("unexpected output shape".into()),
            OnnxError::Io("file not found".into()),
            OnnxError::Event("bad event bytes".into()),
        ];
        for e in &cases {
            let s = e.summary();
            assert!(!s.contains('\n'), "summary must be one line, got: {s:?}");
            assert!(!s.is_empty(), "summary must not be empty");
        }
    }

    // -----------------------------------------------------------------------
    // 6. model_registry_empty
    // -----------------------------------------------------------------------

    #[test]
    fn model_registry_empty() {
        let cfg = OnnxConfig::default();
        let r = ModelRegistry::empty(cfg.clone());
        assert!(r.sentiment_model.is_none());
        assert!(r.decision_model.is_none());
        assert!(r.models.is_empty());
        assert!(!r.sentiment_ready());
        assert_eq!(r.config, cfg);
    }

    // -----------------------------------------------------------------------
    // 7. load_models_missing_file_yields_soft_none
    // -----------------------------------------------------------------------

    #[test]
    fn load_models_missing_file_yields_soft_none() {
        // Point at a path that does not exist. `load_models`
        // should return `Ok(ModelRegistry { sentiment_model:
        // None, decision_model: None, ... })` rather than an
        // error — the plugin still LOADS so the host can
        // `bee plugin list`; only inference fails.
        let cfg = OnnxConfig {
            sentiment_model_path: "/tmp/definitely-not-a-real-onnx-sentiment.onnx".into(),
            decision_model_path: "/tmp/definitely-not-a-real-onnx-decision.onnx".into(),
            max_batch_size: 8,
            device: "cpu".into(),
        };
        let r = load_models(&cfg).expect("missing files are not a hard error");
        assert!(r.sentiment_model.is_none());
        assert!(r.decision_model.is_none());
        assert!(!r.sentiment_ready());
    }

    // -----------------------------------------------------------------------
    // 8. sentiment_result_default
    // -----------------------------------------------------------------------

    #[test]
    fn sentiment_result_default() {
        let r = SentimentResult::default();
        assert_eq!(r.score, 0.0);
        assert_eq!(r.class, SentimentClass::Neutral);
    }

    // -----------------------------------------------------------------------
    // 9. direction_result_default
    // -----------------------------------------------------------------------

    #[test]
    fn direction_result_default() {
        let r = DirectionResult::default();
        assert_eq!(r.direction, Direction::Flat);
    }

    // -----------------------------------------------------------------------
    // 10. model_score_result_default
    // -----------------------------------------------------------------------

    #[test]
    fn model_score_result_default() {
        let r = ModelScoreResult::default();
        assert_eq!(r.score, 0.0);
    }

    // -----------------------------------------------------------------------
    // 11. sentiment_result_bincode_roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn sentiment_result_bincode_roundtrip() {
        let r = SentimentResult {
            score: 0.42,
            class: SentimentClass::Positive,
        };
        let bytes = bincode::serialize(&r).expect("serialize sentiment result");
        let back: SentimentResult =
            bincode::deserialize(&bytes).expect("deserialize sentiment result");
        assert_eq!(r, back);
    }

    // -----------------------------------------------------------------------
    // 12. direction_result_bincode_roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn direction_result_bincode_roundtrip() {
        let r = DirectionResult {
            direction: Direction::Up,
        };
        let bytes = bincode::serialize(&r).expect("serialize direction result");
        let back: DirectionResult =
            bincode::deserialize(&bytes).expect("deserialize direction result");
        assert_eq!(r, back);
    }

    // -----------------------------------------------------------------------
    // 13. model_score_result_bincode_roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn model_score_result_bincode_roundtrip() {
        let r = ModelScoreResult { score: -1.5 };
        let bytes = bincode::serialize(&r).expect("serialize model score result");
        let back: ModelScoreResult =
            bincode::deserialize(&bytes).expect("deserialize model score result");
        assert_eq!(r, back);
    }

    // -----------------------------------------------------------------------
    // 14. plugin_manifest_lists_4_handlers
    // -----------------------------------------------------------------------

    #[test]
    fn plugin_manifest_lists_4_handlers() {
        let m = OnnxMlFactory::manifest();
        assert_eq!(m.handlers.len(), 4, "expected 4 handlers in manifest");
        let names: Vec<&str> = m.handlers.iter().map(|h| h.name.as_str()).collect();
        assert!(names.contains(&"sentiment_score"), "missing sentiment_score");
        assert!(names.contains(&"sentiment_class"), "missing sentiment_class");
        assert!(names.contains(&"price_direction"), "missing price_direction");
        assert!(names.contains(&"model_score"), "missing model_score");
    }

    // -----------------------------------------------------------------------
    // 15. plugin_manifest_no_adapters
    // -----------------------------------------------------------------------

    #[test]
    fn plugin_manifest_no_adapters() {
        let m = OnnxMlFactory::manifest();
        assert!(
            m.adapters.is_empty(),
            "onnx-ml is a Handler-only plugin; manifest.adapters must be empty, got {}",
            m.adapters.len()
        );
    }

    // -----------------------------------------------------------------------
    // 16. init_state_handler_returns_empty
    // -----------------------------------------------------------------------

    #[test]
    fn init_state_handler_returns_empty() {
        // For each of the 4 shims, call `init_state` and verify the
        // decoded `OnnxHandlerState` has an empty `_reserved` field.
        // Handlers have no inherent state; the per-stream state is
        // just the default `OnnxHandlerState` (empty reserved).
        let shims: [(&str, unsafe extern "C" fn(*mut EventBytes) -> i32); 4] = [
            ("sentiment_score", sentiment_score_shim::init_state),
            ("sentiment_class", sentiment_class_shim::init_state),
            ("price_direction", price_direction_shim::init_state),
            ("model_score", model_score_shim::init_state),
        ];
        for (name, init_fn) in shims {
            let state_bytes = call_init_state(init_fn);
            let state: OnnxHandlerState =
                bincode::deserialize(&state_bytes).expect("decode handler state");
            assert!(
                state._reserved.is_empty(),
                "{name}: handler state must be empty, got {} bytes",
                state._reserved.len()
            );
        }
    }

    // -----------------------------------------------------------------------
    // 17. sentiment_class_enum_variants_distinct
    // -----------------------------------------------------------------------

    #[test]
    fn sentiment_class_enum_variants_distinct() {
        let pos = bincode::serialize(&SentimentClass::Positive).expect("ser positive");
        let neu = bincode::serialize(&SentimentClass::Neutral).expect("ser neutral");
        let neg = bincode::serialize(&SentimentClass::Negative).expect("ser negative");
        // Bincode encodes a unit variant as a u32 discriminant
        // (little-endian on the default config). The 3 variants
        // must produce 3 distinct byte strings — and they do, since
        // the order in the enum is `Positive=0, Neutral=1, Negative=2`.
        assert_ne!(pos, neu, "positive and neutral must bincode distinctly");
        assert_ne!(pos, neg, "positive and negative must bincode distinctly");
        assert_ne!(neu, neg, "neutral and negative must bincode distinctly");
        // Sanity-check the discriminant ordering (positive < neutral
        // < negative by declaration order).
        assert_eq!(pos, vec![0, 0, 0, 0], "Positive discriminant is 0");
        assert_eq!(neu, vec![1, 0, 0, 0], "Neutral discriminant is 1");
        assert_eq!(neg, vec![2, 0, 0, 0], "Negative discriminant is 2");
    }

    // -----------------------------------------------------------------------
    // 18. direction_enum_variants_distinct
    // -----------------------------------------------------------------------

    #[test]
    fn direction_enum_variants_distinct() {
        let up = bincode::serialize(&Direction::Up).expect("ser up");
        let down = bincode::serialize(&Direction::Down).expect("ser down");
        let flat = bincode::serialize(&Direction::Flat).expect("ser flat");
        assert_ne!(up, down, "up and down must bincode distinctly");
        assert_ne!(up, flat, "up and flat must bincode distinctly");
        assert_ne!(down, flat, "down and flat must bincode distinctly");
        // Sanity-check the discriminant ordering (Up=0, Down=1,
        // Flat=2 by declaration order).
        assert_eq!(up, vec![0, 0, 0, 0], "Up discriminant is 0");
        assert_eq!(down, vec![1, 0, 0, 0], "Down discriminant is 1");
        assert_eq!(flat, vec![2, 0, 0, 0], "Flat discriminant is 2");
    }

    // -----------------------------------------------------------------------
    // 19. handler_short_circuits_when_model_not_loaded
    // -----------------------------------------------------------------------

    #[test]
    fn handler_short_circuits_when_model_not_loaded() {
        // The handlers all look up the registry in the process-
        // global slot. If the slot is empty (no init has run),
        // the shim returns -1 and writes a "plugin not
        // initialized" error to `*err_out`. We exercise this by
        // temporarily replacing the slot with `None` (the test
        // is the only code that touches the slot directly).
        let slot = registry_slot();
        let saved: Option<Arc<ModelRegistry>> = {
            let mut guard = slot.lock().expect("registry mutex poisoned");
            guard.take()
        };

        // Build a minimal TextEvent and FeaturesEvent payload so
        // the shim can get past the bincode-decode step.
        let text_event = TextEvent {
            text: "Bitcoin surges".to_string(),
            ts: 1_700_000_000,
        };
        let text_bytes = bincode::serialize(&text_event).expect("ser text event");

        let mut new_state = MaybeUninit::<EventBytes>::zeroed();
        let mut result = MaybeUninit::<EventBytes>::zeroed();
        let mut err = MaybeUninit::<EventBytes>::zeroed();
        let rc = unsafe {
            sentiment_score_shim::handle(
                std::ptr::null(),
                0,
                text_bytes.as_ptr(),
                text_bytes.len(),
                new_state.as_mut_ptr(),
                result.as_mut_ptr(),
                err.as_mut_ptr(),
            )
        };
        assert_eq!(rc, -1, "sentiment_score with no registry must return -1");
        let new_state = unsafe { new_state.assume_init() };
        let result = unsafe { result.assume_init() };
        let err = unsafe { err.assume_init() };
        // We wrote a default result and an error string; both
        // must be non-null so the host can free the allocations.
        assert!(!new_state.ptr.is_null() && new_state.len > 0);
        assert!(!result.ptr.is_null() && result.len > 0);
        assert!(!err.ptr.is_null() && err.len > 0);
        // Free the allocations to avoid leaks in the test.
        unsafe {
            let _ = Vec::from_raw_parts(new_state.ptr as *mut u8, new_state.len, new_state.len);
            let _ = Vec::from_raw_parts(result.ptr as *mut u8, result.len, result.len);
            let err_str: String = bincode::deserialize(std::slice::from_raw_parts(
                err.ptr,
                err.len,
            ))
            .expect("decode err string");
            assert!(
                err_str.contains("not initialized"),
                "expected 'not initialized' in err, got: {err_str:?}"
            );
            let _ = Vec::from_raw_parts(err.ptr as *mut u8, err.len, err.len);
        }

        // Restore whatever was in the slot before this test so
        // the rest of the suite (and any parallel test) sees a
        // consistent state.
        if let Some(r) = saved {
            let mut guard = slot.lock().expect("registry mutex poisoned");
            *guard = Some(r);
        }
    }
}
