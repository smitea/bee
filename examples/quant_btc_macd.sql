-- Quant strategy 1: BTC K-line + MACD/EMA (technical only).
-- Pipeline:
--   binance.subscribe('BTC/USDT', '5min')  -- producer
--     -> ta.macd(price, 12, 26, 9)  -- Handler: MACD
--     -> ta.ema(price, 20)          -- Handler: EMA
--     -> influxdb.measurement('btc_macd')  -- sink
-- Backfill is opt-in: add `from='2024-06-01'` to the binance call
-- to test ADR-0011 backfill-on-subscribe semantics (separate run).

use binance;
use influxdb;

SELECT
  b.timestamp AS ts,
  b.symbol,
  b.price,
  ta.macd(price=b.price, fast=12, slow=26, signal=9) AS macd,
  ta.ema(price=b.price, period=20) AS ema20
FROM binance.subscribe(symbol='BTC/USDT', interval='5min') AS b
EMIT INTO influxdb.measurement(name='btc_macd', database='quant');
