-- Quant strategy 2: BTC K-line + FinBERT news sentiment + decision tree.
-- Pipeline:
--   binance.subscribe('BTC/USDT', '5min')   -- producer
--   google_news.search('bitcoin')          -- second producer
--     ASOF JOIN b                            -- time-aligned join
--     -> news.sentiment('finbert', headline)  -- Handler: FinBERT
--     -> tree.decide(price, score) -> action  -- Handler: decision tree
--     -> influxdb.measurement('btc_sentiment')

use binance;
use google_news;
use influxdb;

SELECT
  b.timestamp AS ts,
  b.symbol,
  b.price,
  news.sentiment(model='finbert', text=headline) AS score,
  tree.decide(price=b.price, sentiment=score) AS action
FROM binance.subscribe(symbol='BTC/USDT', interval='5min') AS b
  ASOF JOIN google_news.search(query='bitcoin') AS news
EMIT INTO influxdb.measurement(name='btc_sentiment', database='quant');
