# bee-plugin-binance

Production-grade Binance adapter for Bee (S34 in the quant-trading reference
implementation). Implements real WebSocket + REST + backfill-on-subscribe
semantics per [S34 in the quant stories](../../../../docs/best-practices/quant/stories.md#s34--bee-plugin-binance-production-grade-binance-adapter-real-ws--rest--backfill).

## Quick start

1. Build the plugin:
   ```bash
   cargo build --release -p bee-plugin-binance
   ```
   Output: `target/release/libbee_plugin_binance.dylib` (macOS), `.so` (Linux), or `.dll` (Windows).

2. Load it into a running Bee node:
   ```bash
   bee plugin load target/release/libbee_plugin_binance.dylib
   bee plugin list  # verify the plugin is listed with a stable PluginId
   ```

3. Create a Datasource with the connection-level config (see below):
   ```bash
   bee datasource create binance \
     --adapter binance_subscribe \
     --config @binance.example.json
   ```
   The config file is the JSON shown in `.env.example`.

4. Use it in a SQL pipeline:
   ```sql
   use binance;

   CREATE VIEW v_btc AS
   SELECT * FROM binance.subscribe('BTC/USDT', '5min');

   EMIT INTO console SELECT * FROM v_btc LIMIT 5;
   ```

## Datasource config (connection-level only — ADR-0010)

The Datasource config holds only connection-level fields. Per-call args (symbol, interval, from) go in the SQL.

```jsonc
{
  "ws_url":             "wss://stream.binance.com:9443",  // default; admin may override
  "rest_url":           "https://api.binance.com",         // default
  "api_key":            "<from bee secret store; optional for public market data>",
  "api_secret":         "<from bee secret store; optional>",
  "rate_limit_per_sec": 10,                                // per-IP Binance limit
  "tenant":             0                                   // uint16; 0 = global (ADR-0010)
}
```

See `.env.example` for the canonical template.

## Per-call args (in SQL)

```sql
binance.subscribe('BTC/USDT', '5min')                  -- live only
binance.subscribe('BTC/USDT', '5min', from => '2024-01-01')  -- with backfill
```

- `symbol` (e.g. `'BTC/USDT'`, `'ETH/USDT'`)
- `interval` (e.g. `'1m'`, `'5m'`, `'1h'`, `'1d'`) — Binance's native interval codes
- `from` (optional, ISO-8601 timestamp) — triggers backfill if in the past

## Stream identity (ADR-0011)

```
StreamSignature = sha256("binance" || "subscribe" || symbol || interval)
```

The `from` argument is **not** part of the Stream identity. Multiple Subscribers with different `from` values share the same Producer (same Stream signature) but each receives their own backfill range.

This is the same model as Kafka: a topic is identified by `(source, format)`, not by `from` offsets.

## Backfill semantics

When `subscribe(symbol, interval, from)` is called:

1. Read the Producer's high-water mark `H` from KV (`state/producer/<stream_id>/hwm`).
2. If `from < H`: call REST `GET /api/v3/klines?symbol=...&interval=...&startTime=from&endTime=H&limit=1000` and emit the K-lines in time order, tagged with the offset.
3. If `from >= H` or `from` is null: skip backfill; go straight to WS subscription.
4. Subscribe to WS `/ws/<symbol>@kline_<interval>` and emit new K-lines as they arrive.
5. The Subscriber's Task State stores the last-consumed offset; on restart, the Subscriber rejoins the Stream and asks for backfill from its own offset (independent of the Producer's HWM).

REST pagination: Binance returns ≤ 1000 K-lines per request. The plugin loops, advancing `startTime` to the last emitted K-line's `close_time + 1ms` until the desired `to` is reached.

## Rate limiting

A simple token-bucket rate limiter (default 10 requests/second) wraps all REST calls. WS subscriptions are not rate-limited (Binance allows unlimited WS streams per IP).

To tune, set `rate_limit_per_sec` in the Datasource config.

## Credentials

- For MVP, the plugin reads `api_key` / `api_secret` from the Datasource config (which references the Bee secret store).
- 1.x: replace with Vault / AWS Secrets Manager (out of scope).
- The plugin does **not** fall back to environment variables — config is the single source of truth.

## Building

```bash
cargo build --release -p bee-plugin-binance
```

## Testing

```bash
cargo test -p bee-plugin-binance
```

The unit tests cover:
- `stream_signature` (does NOT include `from`)
- `BinanceConfig` default values + bincode round-trip
- `KlineEvent` bincode round-trip
- Backfill decision logic (`decide_backfill` pure function)
- Rate limiter respects `rate_limit_per_sec`

Live network tests (against real Binance WS + REST) are a follow-up.
