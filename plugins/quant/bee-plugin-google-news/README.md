# bee-plugin-google-news

Production-grade NewsAPI adapter for Bee (S35 in the quant-trading reference
implementation). Implements real HTTP polling against the NewsAPI `/v2/everything`
and `/v2/top-headlines` endpoints, with rate limiting and Stream identity per
[S35 in the quant stories](../../../../docs/best-practices/quant/stories.md#s35--bee-plugin-google-news-production-grade-newsapi-adapter-real-http).

## Quick start

1. Build the plugin:
   ```bash
   cargo build --release -p bee-plugin-google-news
   ```
   Output: `target/release/libbee_plugin_google_news.dylib` (macOS), `.so` (Linux), or `.dll` (Windows).

2. Load it into a running Bee node:
   ```bash
   bee plugin load target/release/libbee_plugin_google_news.dylib
   bee plugin list  # verify the plugin is listed
   ```

3. Create a Datasource with the connection-level config (see `.env.example`):
   ```bash
   bee datasource create google_news \
     --adapter google_news_search \
     --config @google_news.example.json
   ```
   The config file is the JSON shown in `.env.example`. **Note**: `api_key` is required.

4. Use it in a SQL pipeline:
   ```sql
   use google_news;

   CREATE VIEW v_btc_news AS
   SELECT * FROM google_news.search('Bitcoin', from => '2024-06-01', sort_by => 'publishedAt');

   EMIT INTO console SELECT title, url FROM v_btc_news LIMIT 10;
   ```

## Datasource config (connection-level only — ADR-0010)

The Datasource config holds only connection-level fields. Per-call args (query, from, to, sort_by, page_size) go in the SQL.

```jsonc
{
  "api_key":            "<from bee secret store; required>",
  "base_url":           "https://newsapi.org/v2",
  "rate_limit_per_sec": 5,    // NewsAPI free tier: 100/day; pro depends on plan
  "language":           "en", // default
  "tenant":             0
}
```

See `.env.example` for the canonical template.

## Per-call args (in SQL)

```sql
google_news.search('Bitcoin')                                       -- minimal
google_news.search('Bitcoin', from => '2024-06-01', sort_by => 'publishedAt')  -- with time window + sort
google_news.search('AAPL OR "Apple Inc"', page_size => 50)          -- complex query
google_news.top_headlines('tech')                                  -- top headlines
google_news.top_headlines(country => 'us', category => 'business')  -- top headlines by country + category
```

- `query` (e.g. `'Bitcoin'`, `'AAPL OR "Apple Inc"'`)
- `from` / `to` (ISO-8601 timestamps; required for non-headlines endpoints)
- `sort_by` (`'publishedAt'` | `'relevancy'` | `'popularity'`)
- `page_size` (default 100, max 100; clamped server-side per NewsAPI's spec)
- For `top_headlines`: `country` (e.g. `'us'`), `category` (e.g. `'business'`)

## Stream identity

```
StreamSignature = sha256("google_news" || method || query)
```

The `from` / `to` / `sort_by` arguments are **not** part of the Stream identity. Multiple Subscribers with different time windows share the same Producer (same Stream signature) but each receives their own polled results.

## Polling cadence

The plugin polls at a configurable cadence (default 60 seconds). To tune, add `poll_interval_secs` to the `search`/`top_headlines` per-call args:

```sql
google_news.search('Bitcoin', poll_interval_secs => 30)  -- poll every 30 seconds
```

## Rate limiting

A simple token-bucket rate limiter (default 5 requests/second) wraps all REST calls. To tune, set `rate_limit_per_sec` in the Datasource config.

## Credentials

- The plugin reads `api_key` from the Datasource config (which references the Bee secret store).
- 1.x: replace with Vault / AWS Secrets Manager (out of scope).
- The plugin does **not** fall back to environment variables — config is the single source of truth.

## Building

```bash
cargo build --release -p bee-plugin-google-news
```

## Testing

```bash
cargo test -p bee-plugin-google-news
```

The unit tests cover:
- `stream_signature` (does NOT include `from`/`to`/`sort_by`)
- `GoogleNewsConfig` default values + bincode round-trip
- `ArticleEvent` and `SearchArgs` bincode round-trip
- URL building (with/without optional params, URL encoding of special chars)
- Rate limiter respects `rate_limit_per_sec`
- `page_size` clamping (max 100 per NewsAPI spec)

Live network tests (against real NewsAPI) are a follow-up.
