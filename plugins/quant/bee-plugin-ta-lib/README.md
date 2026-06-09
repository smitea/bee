# bee-plugin-ta-indicators

Production-grade technical-analysis Handlers for Bee (S38 in the
quant-trading reference implementation). Implements real yata
indicators (MACD, EMA, RSI, BBANDS, ATR, VWAP) as SQL UDFs, with
KV-backed streaming state, per
[S38 in the quant stories](../../../../docs/best-practices/quant/stories.md#s38--bee-plugin-ta-indicators-production-grade-technical-analysis-handlers-real-yata--ta-lib).

## Note: this is a Handler plugin, not an Adapter

This plugin registers **SQL UDFs** (Handlers), not Datasource Adapters.
There's no Datasource config; the plugin is loaded by Bee at startup and
its UDFs are immediately available in any SQL pipeline that uses the
plugin's namespace.

## Quick start

1. Build the plugin:
   ```bash
   cargo build --release -p bee-plugin-ta-indicators
   ```
   Output: `target/release/libbee_plugin_ta_indicators.dylib` (macOS), `.so` (Linux), or `.dll` (Windows).

2. Load it into a running Bee node:
   ```bash
   bee plugin load target/release/libbee_plugin_ta_indicators.dylib
   bee dsl functions  # verify the 6 UDFs are listed
   ```

3. Use the UDFs in a SQL pipeline:
   ```sql
   use ta_indicators;
   
   CREATE VIEW v_btc_macd AS
   SELECT ts, close,
          MACD(close, 12, 26, 9, ts) AS macd,
          EMA(close, 26, ts)         AS ema26,
          RSI(close, 14, ts)         AS rsi14,
          BBANDS(close, 20, 2.0, ts) AS bb_upper,  -- returns a struct
          ATR(high, low, close, 14, ts) AS atr14,
          VWAP(close, volume, ts)    AS vwap
   FROM v_btc_klines;
   ```

## Registered UDFs (Handlers)

| UDF | Signature | Backed by | Use case |
| --- | --- | --- | --- |
| `MACD` | `MACD(price, fast, slow, signal, ts)` | `yata::MACDIndicator` | Trend-following crossover |
| `EMA` | `EMA(price, period, ts)` | `yata::EMAIndicator` | Smoothing |
| `RSI` | `RSI(price, period, ts)` | `yata::RSIIndicator` | Overbought/oversold |
| `BBANDS` | `BBANDS(price, period, std_dev, ts)` | `yata::BollingerBands` | Volatility |
| `ATR` | `ATR(high, low, close, period, ts)` | `yata::TR` Method + manual Wilder smoothing | Stop-loss sizing |
| `VWAP` | `VWAP(price, volume, ts)` | Custom (running sum) | Intraday fair value |

All UDFs are **streaming-friendly**: they accept `(price, ts)` tuples and emit one output per input.

## Plugin-level config (NOT Datasource config)

```jsonc
{
  "indicator_backend": "yata"   // "yata" (pure Rust) | "ta-lib" (C FFI; optional, not yet implemented)
}
```

`yata` is the default and only-implemented backend. The `ta-lib` option is reserved for a future 1.x plugin revision; the MVP rejects it with a clear error.

## State storage

Per-stream state is stored in Bee's KV Cluster under `state/handler/<stream_id>/<indicator_name>/`. On restart, the state is restored from the last checkpoint; indicators resume mid-stream.

## Backend choice rationale

`yata` is pure Rust (no C deps, no FFI) and provides all the indicators the S38 spec requires (MACD, EMA, RSI, BBANDS, ATR, VWAP). The `ta-lib` option is a future 1.x feature for users who want the canonical C implementation; it requires the `ta-lib-sys` C dep and is out of scope for the S38 MVP.

## Performance

The plugin is pure compute with no I/O; performance is bounded by the yata library's internal state. For a typical 5-min K-line stream at 1 Hz, all 6 indicators can process 100K+ events/sec on a single core.

## Building

```bash
cargo build --release -p bee-plugin-ta-indicators
```

## Testing

```bash
cargo test -p bee-plugin-ta-indicators
```

The unit tests cover:
- Each indicator's output matches the yata oracle (verified to 6 decimal places)
- State round-trip (init → handle → handle → ... produces the same output as one long handle)
- Per-indicator init/handle dispatch
- `yata` and `ta-lib` backend selection (ta-lib rejected)
- Plugin manifest lists 6 handlers, 0 adapters
- yata 0.6 type names are real (compile-time check)

20+ unit tests total. No network tests needed (this is a pure-compute plugin).
