-- quant_btc_strategy.sql — S40 canonical e2e pipeline.
-- See docs/best-practices/quant/stories.md#s40 for the full spec.
--
-- This is the production reference pipeline: 6 production plugins
-- (S34-S39), one binance WS feed, one Google News sentiment feed,
-- ASOF JOIN, technical indicators (MACD / EMA / RSI), a price_direction
-- Handler, and dual sinks (InfluxDB time-series + MongoDB trade log).
--
-- Two other variants live alongside this file:
--   * quant_btc_strategy_backfill.sql — same as this, but the binance
--     call adds `from => '2024-06-01'` to warm up state from history.
--   * quant_btc_strategy_v2.sql      — a second strategy on the same
--     Datasource / Stream; demonstrates Producer sharing.

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

CREATE VIEW v_final_decision AS
SELECT
    ts,
    price_direction(
        struct_pack(
            ema26      AS ema26,
            rsi14      AS rsi14,
            macd       AS macd,
            sentiment  AS sentiment
        )
    )                                                       AS direction,
    close,
    sentiment
FROM v_decision_input;

EMIT INTO influxdb.write(
    'klines',
    tag_cols   => ARRAY['symbol'],
    field_cols => ARRAY['close', 'volume', 'macd', 'rsi14']
)
SELECT ts, symbol, close, volume, macd, rsi14 FROM v_btc_metrics;

EMIT INTO mongodb.insert('trades',
    struct_pack(direction, close, sentiment, ts)
)
SELECT direction, close, sentiment, ts
FROM v_final_decision
WHERE direction IS NOT NULL;
