-- quant_btc_strategy_backfill.sql — S40 backfill variant.
-- See docs/best-practices/quant/stories.md#s40 for the full spec.
--
-- Identical to quant_btc_strategy.sql EXCEPT the binance call adds
--   from => '2024-06-01'
-- which triggers the S34 backfill-on-subscribe path: the Producer
-- first emits historical K-lines from 2024-06-01 to the high-water
-- mark (HWM), then seamlessly transitions to the live WebSocket.
-- Use this pipeline as a "warm up" step at deploy time so that MACD
-- / EMA / RSI have enough history to produce stable values before
-- the first live tick.

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
FROM binance.subscribe('BTC/USDT', '5min', from => '2024-06-01');

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
