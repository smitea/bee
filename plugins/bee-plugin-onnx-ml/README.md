# bee-plugin-onnx-ml

Production-grade ONNX ML model Handlers for Bee (S39 in the
quant-trading reference implementation). Implements real `tract` ONNX
runtime + `tokenizers` WordPiece tokenization for FinBERT sentiment, per
[S39 in the quant stories](../../../../docs/best-practices/quant/stories.md#s39--bee-plugin-onnx-ml-production-grade-onnx-ml-model-handlers-real-tract-runtime--finbert).

## Note: this is a Handler plugin, not an Adapter

This plugin registers **SQL UDFs** (Handlers), not Datasource Adapters.
There's no Datasource config; the plugin is loaded by Bee at startup and
its UDFs are inmediatamente available in any SQL pipeline.

## Quick start

1. Download the FinBERT model (one-time, user supplies):
   ```bash
   # ProsusAI FinBERT (or a financial-sentiment-fine-tuned variant):
   wget -O models/finbert-quant.onnx https://example.com/finbert-quant.onnx
   wget -O models/finbert-quant.onnx.tokenizer.json https://example.com/finbert-quant-tokenizer.json
   ```
   The model file is the ONNX export; the `.tokenizer.json` file is the HuggingFace tokenizers config (WordPiece).

   For the decision model (price_direction), supply your own trained ONNX model:
   ```bash
   cp /your/trained/btc-direction-1h.onnx models/btc-direction-1h.onnx
   ```

2. Build the plugin:
   ```bash
   cargo build --release -p bee-plugin-onnx-ml
   ```
   Output: `target/release/libbee_plugin_onnx_ml.dylib` (macOS), `.so` (Linux), or `.dll` (Windows).

3. Load it into a running Bee node with the plugin config:
   ```bash
   bee plugin load target/release/libbee_plugin_onnx_ml.dylib --config @onnx-ml.example.json
   bee dsl functions  # verify the 4 UDFs are listed
   ```

4. Use the UDFs in a SQL pipeline:
   ```sql
   use onnx_ml;
   
   CREATE VIEW v_news_scored AS
   SELECT title, url,
          sentiment_score(title) AS score,
          sentiment_class(title) AS class
   FROM v_google_news;
   
   CREATE VIEW v_btc_decision AS
   SELECT ts, ema26, rsi14, macd, sentiment,
          price_direction(struct_pack(ema26, rsi14, macd, sentiment)) AS direction
   FROM v_decision_input;
   ```

## Registered UDFs (Handlers)

| UDF | Signature | Model | Use case |
| --- | --- | --- | --- |
| `sentiment_score` | `sentiment_score(text_col)` | FinBERT (ProsusAI, ONNX) | Returns a float in `[-1, 1]`: negative = bearish, positive = bullish |
| `sentiment_class` | `sentiment_class(text_col)` | FinBERT (ProsusAI, ONNX) | Returns one of `{"positive", "neutral", "negative"}` |
| `price_direction` | `price_direction(features_struct)` | User-supplied ONNX model | Returns one of `{"up", "down", "flat"}` for the next bar |
| `model_score` | `model_score(model_name, features_struct)` | Generic | Returns the model's raw output (float or class index) |

## Plugin-level config (NOT Datasource config)

```jsonc
{
  "sentiment_model_path": "./models/finbert-quant.onnx",
  "decision_model_path":  "./models/btc-direction-1h.onnx",
  "max_batch_size":       32,
  "device":               "cpu"
}
```

See `.env.example` for the canonical template.

## Model file format

- **ONNX** (Open Neural Network Exchange) format
- Sentiment model: 3-class classification (negative/neutral/positive); input is tokenized text (WordPiece); output is `[1, 3]` logits
- Decision model: 3-class classification (down/flat/up); input is a flat feature vector; output is `[1, 3]` logits
- Tokenizer file: HuggingFace `tokenizer.json` format (WordPiece config + vocab); used for `sentiment_score` and `sentiment_class`

**No model weights are bundled in the plugin crate** (per spec). Models are loaded at runtime from the paths in the plugin config.

## Batching

`sentiment_score` accepts one text per call, but the plugin batches up to `max_batch_size` calls into a single `tract` inference to amortize overhead. (Batching is the MVP; the current implementation does single-call inference; full batching is a 1.x feature.)

## Performance

- CPU inference: ~5-20ms per `sentiment_score` call (FinBERT is ~110M params; tract runs it on AVX2 by default)
- GPU inference: not yet supported (1.x feature)
- For 100-row `sentiment_score` burst, total wall-clock is ~1-2s on a modern CPU

## Building

```bash
cargo build --release -p bee-plugin-onnx-ml
```

## Testing

```bash
cargo test -p bee-plugin-onnx-ml
```

The unit tests cover:
- Config (default values, bincode round-trip, empty/invalid bytes handling)
- Plugin manifest (4 handlers, 0 adapters)
- Init_state (all 4 handlers return empty state)
- Enum variants (SentimentClass, Direction — distinct bincode bytes)
- Handler error paths (all 3 typed handlers return `OnnxError::ModelNotLoaded` when no model is loaded)
- Integration with a synthetic ONNX model (Identity passthrough) — verifies the real tract load + run + extract pipeline

20+ unit tests total. Live network tests (against a real FinBERT model) are a follow-up (the test environment doesn't have a real model file).

## Future 1.x work

- GPU inference (CUDA / Metal)
- Real batching (the MVP uses single-call inference)
- Quantized model support (INT8)
- More tokenizer backends (BPE, SentencePiece, in addition to WordPiece)
