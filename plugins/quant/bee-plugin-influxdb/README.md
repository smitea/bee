# bee-plugin-influxdb

Production-grade InfluxDB v2 Output Adapter for Bee (S36 in the
quant-trading reference implementation). Implements real InfluxDB v2
line protocol writer (batched HTTP POST) + Flux query poller, with
batching + rate limiting, per
[S36 in the quant stories](../../../../docs/best-practices/quant/stories.md#s36--bee-plugin-influxdb-production-grade-influxdb-v2-output-adapter-real-line-protocol).

## Quick start

1. Build the plugin:
   ```bash
   cargo build --release -p bee-plugin-influxdb
   ```
   Output: `target/release/libbee_plugin_influxdb.dylib` (macOS), `.so` (Linux), or `.dll` (Windows).

2. Load it into a running Bee node:
   ```bash
   bee plugin load target/release/libbee_plugin_influxdb.dylib
   bee plugin list  # verify the plugin is listed
   ```

3. Create a Datasource with the connection-level config (see `.env.example`):
   ```bash
   bee datasource create influxdb \
     --adapter influxdb_write \
     --config @influxdb.example.json
   ```
   The config file is the JSON shown in `.env.example`. **Note**: `token` and `org` are required.

4. Use it in a SQL pipeline:
   ```sql
   use influxdb;
   
   EMIT INTO influxdb.write(
     'klines',
     tag_cols   => ARRAY['symbol'],
     field_cols => ARRAY['price', 'volume'],
     timestamp_col => 'ts'
   )
   SELECT ts, symbol, price, volume FROM v_btc_metrics;
   ```

## Datasource config (connection-level only — ADR-0010)

```jsonc
{
  "url":        "http://localhost:8086",  // admin-supplied
  "token":      "<from bee secret store; required>",
  "org":        "<InfluxDB org; required>",
  "bucket":     "<default bucket; can be overridden per-call>",
  "timeout_ms": 5000,
  "tenant":     0
}
```

See `.env.example` for the canonical template.

## Per-call args (in SQL — used in `EMIT INTO influxdb.write(...)`)

- `measurement` (e.g. `'klines'`, `'sentiment'`)
- `bucket` (optional override of Datasource default)
- `tag_cols` (array of column names to use as InfluxDB tags)
- `field_cols` (array of column names to use as InfluxDB fields; default = all non-tag numeric columns)
- `timestamp_col` (default `ts`)

## Line protocol mapping

The plugin encodes each emitted event as a single InfluxDB v2 line-protocol row:

```
<measurement>,<tag1>=<v1>,<tag2>=<v2> <field1>=<v1>,<field2>=<v2> <timestamp_ns>
```

Example: an event with `measurement='klines'`, `tags={'symbol': 'BTC/USDT'}`, `fields={'price': 50000.0, 'volume': 1.5}`, `timestamp_ns=1234567890` becomes:

```
klines,symbol=BTC\/USDT price=50000,volume=1.5 1234567890
```

(Tag keys + values are escaped per the InfluxDB spec: `,` → `\,`, ` ` → `\ `, `=` → `=` is preserved.)

## Batching

The plugin buffers emitted events and flushes in batches:
- **Size trigger**: when the buffer reaches `max_batch_size` (default 500 lines), flush immediately.
- **Time trigger**: every `flush_interval_ms` (default 1000ms), flush the current buffer.

Each flush is a single `POST /api/v2/write?org=...&bucket=...` with the line-protocol body.

## Rate limiting

A token-bucket rate limiter (default 100 requests/second) wraps all HTTP calls. To tune, add `rate_limit_per_sec` to the Datasource config.

## Query method (Input adapter for backfill / backtest)

The plugin also registers an Input adapter for Flux queries:

```sql
SELECT * FROM influxdb.query(
  'from(bucket: "quant") |> range(start: -1h) |> filter(fn: (r) => r._measurement == "klines")',
  bucket => 'quant'
);
```

This polls at a configurable cadence and emits the result rows.

## Stream identity

- For `write`: Output adapters don't produce Streams; the signature is `(influxdb, write)` — connection-level only.
- For `query`: `StreamSignature = sha256("influxdb" || "query" || bucket || hash(flux_query))` — different queries are different Producers.

## Credentials

- The plugin reads `token` from the Datasource config (which references the Bee secret store).
- 1.x: replace with Vault / AWS Secrets Manager (out of scope).
- **The token is NEVER logged or included in error messages** (per spec).
- The plugin does **not** fall back to environment variables — config is the single source of truth.

## Building

```bash
cargo build --release -p bee-plugin-influxdb
```

## Testing

```bash
cargo test -p bee-plugin-influxdb
```

The unit tests cover:
- Line-protocol encoding (single row, no tags, string fields, escape rules)
- `InfluxdbConfig` default values + bincode round-trip
- `WriteArgs` bincode round-trip
- Batching flushes at `max_batch_size` (1000-row burst → ≤ 2 flushes)
- Bucket override (per-call `bucket` takes precedence over Datasource default)
- Rate limiter respects `rate_limit_per_sec`
- `write_signature` is the constant `"influxdb:write"`
- `query_signature` is bucket + flux-query dependent
- Token never appears in error messages

Live network tests (against a real InfluxDB v2 instance) are a follow-up.
