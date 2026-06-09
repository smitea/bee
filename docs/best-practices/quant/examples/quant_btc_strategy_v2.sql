-- quant_btc_strategy_v2.sql — S40 second strategy on the SAME binance
-- Stream as quant_btc_strategy.sql.
-- See docs/best-practices/quant/stories.md#s40 for the full spec.
--
-- What this demonstrates:
--   * The `binance` Datasource and the `binance.subscribe('BTC/USDT',
--     '5min')` Stream are shared with quant_btc_strategy.sql — the
--     cluster will run exactly ONE binance Producer Pipeline and
--     fan its output to both strategies (ADR-0003, ADR-0010,
--     ADR-0011).
--   * A different decision function (`model_score` from
--     bee-plugin-onnx-ml) and a different filter (close > 50000,
--     only trade meaningful BTC moves) make this a genuinely
--     independent strategy.
--
-- Run after quant_btc_strategy_backfill.sql so the binance Producer
-- is already warm.

use binance;
use google_news;
use influxdb;
use mongodb;
use ta_indicators;
use onnx_ml;

CREATE VIEW v_btc_metrics AS
SELECT
    open_time                                                       AS ts,
    symbol,
    close,
    volume,
    MACD(close, 12, 26, 9, open_time)                               AS macd,
    EMA(close, 26, open_time)                                       AS ema26,
    RSI(close, 14, open_time)                                       AS rsi14
FROM binance.subscribe('BTC/USDT', '5min');

CREATE VIEW v_btc_sentiment AS
SELECT
    published_at                                                    AS ts,
    sentiment_score(description)                                    AS sentiment,
    title,
    url
FROM google_news.search('Bitcoin', sort_by => 'publishedAt');

CREATE VIEW v_decision_input AS
SELECT
    p.ts,
    p.close,
    p.macd,
    p.rsi14,
    s.sentiment
FROM v_btc_metrics      p
ASOF JOIN v_btc_sentiment s
  ON p.ts >= s.ts;

-- v2 only trades when BTC is in a meaningful range (close > 50000).
-- Different filter than v1.
CREATE VIEW v_filtered_input AS
SELECT *
FROM v_decision_input
WHERE close > 50000;

CREATE VIEW v_final_decision AS
SELECT
    ts,
    -- Different decision: ONNX model from bee-plugin-onnx-ml
    -- (`btc-direction-1h` is a 1h directional model trained on
    -- technical + sentiment features). model_score returns a
    -- signed score in [-1.0, 1.0]: positive = long, negative = short.
    model_score(
        'btc-direction-1h',
        struct_pack(
            ema26      AS ema26,
            rsi14      AS rsi14,
            macd       AS macd,
            sentiment  AS sentiment
        )
    )                                                       AS direction,
    close,
    sentiment
FROM v_filtered_input;

EMIT INTO influxdb.write(
    'klines_v2',
    tag_cols   => ARRAY['symbol'],
    field_cols => ARRAY['close', 'volume', 'macd', 'rsi14']
)
SELECT ts, symbol, close, volume, macd, rsi14 FROM v_btc_metrics;

EMIT INTO mongodb.insert('trades_v2',
    struct_pack(direction, close, sentiment, ts)
)
SELECT direction, close, sentiment, ts
FROM v_final_decision
WHERE direction IS NOT NULL;
