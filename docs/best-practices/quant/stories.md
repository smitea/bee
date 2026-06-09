# Bee · Best Practices · Quant Trading · Stories

This is the quant-trading reference implementation story set
(S33 HITL milestone + S34–S40 production plugins + e2e deploy).
It complements the main repo's `docs/stories.md` (which covers
S0–S31, S41 — the generic Bee feature set + performance showcase).

Cross-references in these stories point back to the main repo's
`docs/stories.md` (e.g., "see S17" means "see main stories.md §S17").

---

### S33 · Quant trading HITL milestone: production deployment with real external systems

- **Type**: **HITL** (umbrella milestone — marked done only after S40 is delivered AND the seed user signs off)
- **Blocked by**: S40
- **ADRs**: 0001, 0003, 0009, 0010, 0011
- **HITL review milestone**: when the production pipeline has been running real money signals for ≥ 1 trading day without manual intervention, schedule a 60-minute walkthrough with the first seed user. They sign off (or note gaps) before S33 is marked done.

> **Why this story exists**: S33 is the **end-to-end production validation** of the architecture. All previous stories (S00–S32) prove mechanisms in isolation. S33 proves they compose under real-world load, real credentials, real network, real third-party rate limits. It produces the "first production deployment" that anchors the 1.0 narrative.

**Status as of 2026-06-08**: all upstream stories are done. S34–S39 (the 6 production plugins) shipped; S40 (the e2e deploy) shipped. **Awaiting HITL review.**

| Upstream | Status | Commit |
| --- | --- | --- |
| S34 (binance production) | ✓ done | pushed to `origin/main` |
| S35 (google_news production) | ✓ done | pushed to `origin/main` |
| S36 (influxdb production) | ✓ done | pushed to `origin/main` |
| S37 (mongodb production) | ✓ done | pushed to `origin/main` |
| S38 (ta-indicators production) | ✓ done | pushed to `origin/main` |
| S39 (onnx-ml production) | ✓ done (real tract + tokenizers) | pushed to `origin/main` |
| S40 (e2e deploy) | ✓ done (3 SQL pipelines + demo script + docs) | pushed to `origin/main` |

**HITL prerequisites** (all met):

- [x] All 6 production plugin crates build independently via `cargo build --release`
- [x] Each plugin's `.so`/`.dylib` is a separate file; one plugin's failure does not block the others
- [x] `bee plugin list` shows all 6 plugins with distinct `PluginId` (sha256 hashes) and their declared `abi_version`
- [x] All 4 Datasource registrations via `bee datasource create` succeed; the configs contain **only** connection-level fields
- [x] `examples/quant_btc_strategy.sql` + `_backfill.sql` + `_v2.sql` all exist under `docs/best-practices/quant/examples/`
- [x] `scripts/demo-quant-prod.sh` exists and is executable
- [x] `scripts/.env.example` documents the required env vars

**Outstanding for HITL sign-off** (the seed user must run these):

- [ ] Run `scripts/demo-quant-prod.sh` with real credentials in `scripts/.env` (Binance API key, NewsAPI key, InfluxDB URL+token+org, MongoDB URI)
- [ ] Deploy the 3 SQL pipelines (canonical, backfill, v2) to a production cluster
- [ ] Let the pipeline run for ≥ 1 trading day (24 hours) with real Binance WS + NewsAPI polling
- [ ] Verify InfluxDB `klines` measurement has live data; verify MongoDB `trades` collection has decision records
- [ ] Verify Producer sharing: both `_strategy` and `_v2` reference the same `binance` Datasource; `bee jobs list` shows 1 `binance` Producer
- [ ] Verify failover: kill a Node hosting the `binance` Producer; both strategies should recover within 1 Orphaned period (≤ 30s)
- [ ] Check the 12 ADR-acceptance items from S40 (data still flows P2P, control still goes through Raft, shared Stream serves both strategies, ...)

**Sign-off form** (filled in by the seed user):

| Field | Value |
| --- | --- |
| Seed user name | _________________ |
| Date of review | _________________ |
| Walkthrough duration (minutes) | _________________ |
| Real money signals observed (Y/N) | _________________ |
| InfluxDB data verified (Y/N) | _________________ |
| MongoDB data verified (Y/N) | _________________ |
| Producer sharing verified (Y/N) | _________________ |
| Failover verified (Y/N) | _________________ |
| ADR-0001 (P2P + Raft) verified (Y/N) | _________________ |
| ADR-0003 (shared Stream) verified (Y/N) | _________________ |
| ADR-0005 (cdylib ABI) verified (Y/N) | _________________ |
| ADR-0006 (SQL extensions: ASOF JOIN, EMIT INTO, UDFs) verified (Y/N) | _________________ |
| ADR-0009 (multi-version) verified (Y/N) | _________________ |
| ADR-0010 (per-call args + Provider/Stream separation) verified (Y/N) | _________________ |
| ADR-0011 (Stream identity + backfill-on-subscribe) verified (Y/N) | _________________ |
| Gaps / new stories (if any) | _________________ |
| **Sign-off (S33 done?)** | **Y / N** |

**Deliverables**

- All 6 production plugin stories (S34–S39) done; all 6 plugins load cleanly in the production cluster ✓
- S40 production pipeline runs end-to-end with real money signals for ≥ 1 trading day without manual intervention *(awaiting HITL review)*
- Seed user review notes captured; any gaps filed as new stories or ADR amendments *(will be filled in at HITL review time)*

**After sign-off**: this story is `done`. The S33 milestone is hit. Bee's quant-trading reference implementation is production-validated, and the 1.0 narrative is anchored on the signed-off deployment.

---

### S34 · `bee-plugin-binance`: production-grade Binance adapter (real WS + REST + backfill)

- **Type**: AFK
- **Blocked by**: S00, S05, S29
- **ADRs**: 0005, 0009, 0010, **0011** (Stream identity + backfill semantics — to be created)

**Crate**: `plugins/bee-plugin-binance/` (new workspace member; `crate-type = ["cdylib"]`)

**Why this is its own crate**: per Bee's principle (ADR-0005) and the user's explicit requirement, every external-system adapter ships as an independent `cdylib` — no cross-plugin imports, no business code in core, max reusability for any user that needs Binance data.

**Datasource config (connection-level only — ADR-0010)**

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

**Per-call args (in SQL — never in Datasource config)**

- `symbol` (e.g. `'BTC/USDT'`)
- `interval` (e.g. `'5min'`, `'1h'`)
- `from` (optional ISO-8601 timestamp; if in the past, the plugin backfills before subscribing to live data — see "Backfill semantics" below)

**Adapter contract (real `tokio-tungstenite` WS + `reqwest` REST)**

| Method | Direction | Backed by | Behavior |
| --- | --- | --- | --- |
| `subscribe(symbol, interval, from?)` | Input | WS `/ws/<symbol>@kline_<interval>` | See backfill semantics below. Emits `KlineEvent { open_time, open, high, low, close, volume, close_time, ... }`. |
| `download_history(symbol, interval, from, to)` | Input (also exposed as a public method) | REST `GET /api/v3/klines?symbol=...&interval=...&startTime=...&endTime=...` | Returns historical K-lines as a batch; paginates internally (Binance returns ≤ 1000 per page). Emits them on the same Stream signature as `subscribe`. |
| `unsubscribe(symbol, interval)` | Input | WS unsubscribe message | Stops the subscription; Producer state retains the high-water mark. |

**Stream identity (refines ADR-0003 / 0010; see ADR-0011)**

- `StreamSignature = sha256("binance" || "subscribe" || symbol || interval)` — does **not** include `from`
- The `from` argument is a **per-Subscriber** concern, not a Stream identity
- Multiple Subscribers can each ask for different backfill ranges and still share the same Producer/Stream
- This is the same model as Kafka: a topic is identified by `(source, format)`, not by `from` offsets

**Backfill semantics (the key new behavior)**

When `subscribe(symbol, interval, from)` is called by a Subscriber, the plugin:

1. Reads the Producer's high-water mark `H` from KV (`state/producer/<stream_id>/hwm`)
2. If `from < H`: call `download_history(symbol, interval, from, H)` and emit the resulting K-lines in time order, tagged with the offset
3. If `from >= H` or `from` is null: skip backfill; go straight to WS subscription
4. Subscribe to WS `/ws/<symbol>@kline_<interval>` and emit new K-lines as they arrive
5. The Subscriber's Task State stores the last-consumed offset; on restart, the Subscriber rejoins the Stream and asks for backfill from its own offset (independent of the Producer's HWM)

**Credentials handling**

- For MVP, the plugin reads `api_key` / `api_secret` from the Datasource config (which references the Bee secret store)
- 1.x: replace with Vault / AWS Secrets Manager (out of scope)
- The plugin does **not** fall back to env vars — config is the single source of truth

**Acceptance criteria**

- [ ] `plugins/bee-plugin-binance/` is an independent workspace member; `Cargo.toml` declares `crate-type = ["cdylib"]`
- [ ] Crate depends only on `bee-plugin-sdk`, `tokio`, `tokio-tungstenite`, `reqwest`, `serde`, `bincode` (no Bee core deps)
- [ ] `bee plugin load plugins/bee-plugin-binance/target/release/libbee_plugin_binance.so` succeeds; `bee plugin list` shows it with a stable `PluginId = sha256(binary)`
- [ ] Loading two different versions side-by-side works (ADR-0009 multi-version)
- [ ] `bee datasource create binance --config @binance.example.json` (where `binance.example.json` contains only connection-level config) registers cleanly
- [ ] Strict-mode `use binance;` SQL: `SELECT * FROM binance.subscribe('BTC/USDT', '5min')` compiles (no warnings)
- [ ] Same SQL with `from => '2024-01-01'` also compiles
- [ ] Plugin connects to real `wss://stream.binance.com:9443` and emits live K-lines within 5 seconds of pipeline start
- [ ] `download_history('BTC/USDT', '5min', '2024-01-01', '2024-01-02')` returns the expected K-line batch via REST (verified against Binance docs)
- [ ] **Backfill-on-subscribe**: when `from` is in the past, the plugin emits historical K-lines first, then seamlessly transitions to live WS events — verified by a single ordered stream at the Subscriber
- [ ] Two Subscribers with different `from` values share the same Producer (Stream signature matches), but each receives their own backfill range
- [ ] Restarting a Subscriber mid-stream resumes from its last offset (not from the Producer's HWM)
- [ ] Rate limiter respects `rate_limit_per_sec` (10/s default); no Binance 429s in a 10-minute live test
- [ ] No credentials, URLs, or other config in source code; all from the Datasource config
- [ ] README in the plugin crate documents: required Datasource config, per-call args, Stream identity, backfill behavior, rate-limit semantics

---

### S35 · `bee-plugin-google-news`: production-grade NewsAPI adapter (real HTTP)

- **Type**: AFK
- **Blocked by**: S00, S05, S29
- **ADRs**: 0005, 0009, 0010

**Crate**: `plugins/bee-plugin-google-news/` (independent `cdylib`)

**Datasource config (connection-level only)**

```jsonc
{
  "api_key":            "<from bee secret store; required>",
  "base_url":           "https://newsapi.org/v2",  // default
  "rate_limit_per_sec": 5,                         // NewsAPI free tier: 100/day; pro depends on plan
  "language":           "en",                      // default
  "tenant":             0
}
```

**Per-call args (in SQL — never in Datasource config)**

- `query` (e.g. `'Bitcoin'` or `'AAPL OR "Apple Inc"'`)
- `from` / `to` (ISO-8601; required for non-headlines endpoints)
- `sort_by` (`'publishedAt'` | `'relevancy'` | `'popularity'`)
- `page_size` (default 100, max 100)

**Adapter contract (real `reqwest`)**

| Method | Direction | Backed by | Behavior |
| --- | --- | --- | --- |
| `search(query, from?, to?, sort_by?, page_size?)` | Input | REST `GET /everything?q=...&from=...&to=...&sortBy=...&pageSize=...` | Polls at a configurable cadence (default 60s); emits `ArticleEvent { published_at, source, author, title, description, url, content }`. |
| `top_headlines(query?, country?, category?)` | Input | REST `GET /top-headlines?q=...&country=...&category=...` | Same polling semantics; emits the same `ArticleEvent` shape. |

**Stream identity**

- `StreamSignature = sha256("google_news" || method || query)` — does **not** include `from`/`to`/`sort_by` (those are per-Subscriber)
- Multiple Subscribers with different time windows share the same Producer

**Acceptance criteria**

- [ ] Independent `cdylib` crate, only depends on `bee-plugin-sdk`, `tokio`, `reqwest`, `serde`, `bincode`
- [ ] Loads cleanly; `bee plugin list` shows it
- [ ] `bee datasource create google_news --config @google_news.example.json` registers cleanly
- [ ] `SELECT * FROM google_news.search('Bitcoin', from => '2024-06-01', sort_by => 'publishedAt')` compiles
- [ ] Plugin hits real `https://newsapi.org/v2/everything` and emits parsed articles within 10 seconds
- [ ] Rate limiter respects `rate_limit_per_sec`; no 429s in a 10-minute test
- [ ] Stream sharing: two Subscribers with different `from` share the same Producer
- [ ] Plugin README documents: required Datasource config, per-call args, Stream identity, polling cadence, rate-limit semantics

---

### S36 · `bee-plugin-influxdb`: production-grade InfluxDB v2 Output Adapter (real line protocol)

- **Type**: AFK
- **Blocked by**: S00, S05, S29
- **ADRs**: 0005, 0009, 0010

**Crate**: `plugins/bee-plugin-influxdb/` (independent `cdylib`)

**Datasource config (connection-level only)**

```jsonc
{
  "url":        "http://localhost:8086",    // admin-supplied
  "token":      "<from bee secret store; required>",
  "org":        "<InfluxDB org; required>",
  "bucket":     "<default bucket; can be overridden per-call>",
  "timeout_ms": 5000,
  "tenant":     0
}
```

**Per-call args (in SQL — used in `EMIT INTO influxdb.write(...)`)**

- `measurement` (e.g. `'klines'`, `'sentiment'`)
- `bucket` (optional override of Datasource default)
- `tag_cols` (array of column names to use as InfluxDB tags)
- `field_cols` (array of column names to use as InfluxDB fields; default = all non-tag numeric columns)
- `timestamp_col` (default `ts`)

**Adapter contract (real InfluxDB v2 client over HTTP line protocol)**

| Method | Direction | Backed by | Behavior |
| --- | --- | --- | --- |
| `write(measurement, tag_cols, field_cols?, bucket?, timestamp_col?)` | Output | `POST /api/v2/write?org=...&bucket=...` (line protocol) | Batches events; flushes on size threshold (default 500 lines) or time threshold (default 1s). Emits `WriteResult { bytes_written, lines_written, status }` back to Bee for observability. |
| `query(flux_query, bucket?)` | Input | `POST /api/v2/query?org=...` (Flux) | Polls at a configurable cadence; emits the result rows. Used for the "load back historical InfluxDB data" loop in the backfill / backtest story. |

**Stream identity**

- For `write`: Output adapters don't produce Streams; the signature is `(influxdb, write)` — connection-level only
- For `query`: `StreamSignature = sha256("influxdb" || "query" || bucket || hash(flux_query))` — different queries are different Producers

**Acceptance criteria**

- [ ] Independent `cdylib` crate, only depends on `bee-plugin-sdk`, `tokio`, `reqwest`, `serde`, `bincode`
- [ ] Real InfluxDB v2 line protocol: emitted bytes parse cleanly with `influx-cli` (or `curl /api/v2/query`)
- [ ] `bee datasource create influxdb --config @influxdb.example.json` registers cleanly
- [ ] `EMIT INTO influxdb.write('klines', tag_cols => ARRAY['symbol'], field_cols => ARRAY['price','volume'])` from SQL compiles and runs
- [ ] Batching behavior: 1000-row burst flushes in ≤ 2 batches (verify in test); no events lost
- [ ] Bucket override: per-call `bucket => 'archive'` writes to the right bucket
- [ ] Rate limiter respects token-bucket config; no 429s under normal load
- [ ] Token never logged; never in error messages
- [ ] Plugin README documents: required Datasource config, per-call args, line-protocol mapping, batching, rate-limit semantics

---

### S37 · `bee-plugin-mongodb`: production-grade MongoDB adapter (real driver; per-call collection)

- **Type**: AFK
- **Blocked by**: S00, S05, S29
- **ADRs**: 0005, 0009, 0010

**Crate**: `plugins/bee-plugin-mongodb/` (independent `cdylib`)

**Datasource config (connection-level only — ADR-0010; note: NO `collection` field)**

```jsonc
{
  "uri":       "mongodb://localhost:27017",     // admin-supplied
  "database":  "trading",                        // default DB; collection is per-call
  "username":  "<from bee secret store; optional>",
  "password":  "<from bee secret store; optional>",
  "app_name":  "bee",                            // appears in MongoDB logs
  "tls":       false,
  "tenant":    0
}
```

**Per-call args (in SQL — `collection` is per-call, NOT in Datasource config)**

- `collection` (e.g. `'trades'`, `'order_decision'`, `'news_articles'`) — **per-call, by design (ADR-0010)**
- For `insert` / `insert_many`: `document` (a struct/row)
- For `find`: `filter` (MongoDB filter doc)
- For `update`: `filter`, `update` (MongoDB update doc)
- For `aggregate`: `pipeline` (array of stages)

**Adapter contract (real `mongodb` crate driver)**

| Method | Direction | Backed by | Behavior |
| --- | --- | --- | --- |
| `insert(collection, document)` | Output | `coll.insert_one(doc)` | Inserts a single document; emits `InsertResult { inserted_id, collection }` back to Bee. |
| `insert_many(collection, documents)` | Output | `coll.insert_many(docs)` | Batched insert; emits batched result. |
| `find(collection, filter)` | Input | `coll.find(filter)` | Polls / change-streams the collection; emits `DocumentEvent` per matching doc. |
| `update(collection, filter, update)` | Output | `coll.update_one(filter, update)` | Returns `UpdateResult { matched_count, modified_count, collection }`. |
| `aggregate(collection, pipeline)` | Input | `coll.aggregate(pipeline)` | Emits result rows. |

**Why `collection` is per-call (not in Datasource config)**

- A single MongoDB cluster holds many collections; the same Datasource `mongodb` should be reusable across all of them
- Different `use mongodb;` calls with different `collection` args are different Streams (StreamSignature includes collection)
- This matches the ADR-0010 principle: **Datasource config = connection-level only; per-call args in SQL**

**Stream identity**

- For `find`/`aggregate`: `StreamSignature = sha256("mongodb" || method || database || collection || hash(filter_or_pipeline))` — different filters/pipelines are different Producers
- For `insert`/`update`: Output adapters don't produce Streams; the signature is `(mongodb, write, database, collection)` — connection-level + collection

**Acceptance criteria**

- [ ] Independent `cdylib` crate, only depends on `bee-plugin-sdk`, `tokio`, `mongodb` (the official Rust driver), `bson`, `serde`, `bincode`
- [ ] Connects to a real MongoDB instance (test: `docker run mongo:7`)
- [ ] `bee datasource create mongodb --config @mongodb.example.json` (no `collection` in the config) registers cleanly
- [ ] Strict-mode SQL: `EMIT INTO mongodb.insert('trades', row)` — `collection` is a per-call string arg, **not** a Datasource field
- [ ] `EMIT INTO mongodb.insert('order_decision', row)` — same Datasource `mongodb`, different collection, different Stream
- [ ] Same `mongodb` Datasource reused across 5+ Pipelines with different collections, all sharing the same MongoDB connection (Bee-level pooling)
- [ ] Documents round-trip: `insert` then `find` returns the inserted doc
- [ ] Credentials never logged; never in error messages
- [ ] Plugin README documents: required Datasource config (no `collection` field), per-call args, Stream identity, pooling behavior, change-stream caveats

---

### S38 · `bee-plugin-ta-indicators`: production-grade technical-analysis Handlers (real `yata` / `ta-lib`)

- **Type**: AFK
- **Blocked by**: S00, S05, S15
- **ADRs**: 0005, 0009, 0010

**Crate**: `plugins/bee-plugin-ta-indicators/` (independent `cdylib`)

> **Note**: this is a **Handler** plugin (pure compute), not an Adapter. No Datasource config; the plugin registers a set of SQL UDFs and is loaded by Bee at startup.

**Plugin config (plugin-level, not Datasource)**

```jsonc
{
  "indicator_backend": "yata"   // "yata" (pure Rust) | "ta-lib" (C FFI; optional)
}
```

**Handler contract (real indicator math, not stubs)**

| Function | Signature | Backed by | Use case |
| --- | --- | --- | --- |
| `MACD(price_col, fast, slow, signal, ts_col)` | SQL UDF | `yata::MACDIndicator` (pure Rust) or `ta-lib` (C) | Trend-following crossover |
| `EMA(price_col, period, ts_col)` | SQL UDF | `yata::EMAIndicator` | Smoothing |
| `RSI(price_col, period, ts_col)` | SQL UDF | `yata::RSIIndicator` | Overbought/oversold |
| `BBANDS(price_col, period, std_dev, ts_col)` | SQL UDF | `yata::BollingerBands` | Volatility |
| `ATR(high_col, low_col, close_col, period, ts_col)` | SQL UDF | `yata::ATRIndicator` | Stop-loss sizing |
| `VWAP(price_col, volume_col, ts_col)` | SQL UDF | Custom (running sum) | Intraday fair value |

**State management**

- All indicators are **streaming-friendly**: they accept `(price, ts)` tuples and emit one output per input (no array-bulk mode required for MVP)
- Per-stream state (rolling buffers) is stored in Bee's KV Cluster under `state/handler/<stream_id>/<indicator_name>/`
- On restart, the state is restored from the last checkpoint; indicators resume mid-stream

**Acceptance criteria**

- [ ] Independent `cdylib` crate, only depends on `bee-plugin-sdk`, `yata`, `serde`, `bincode` (and optionally `ta-lib-sys` if backend = `ta-lib`)
- [ ] `bee plugin load` succeeds; UDFs appear in `bee dsl functions` list
- [ ] `MACD(close, 12, 26, 9, ts)` on a real 5-min BTC stream produces the expected values (validated against `pandas-ta` reference output in tests)
- [ ] `EMA(close, 26, ts)` matches `pandas.Series.ewm(span=26).mean()` to 6 decimal places
- [ ] State is restored correctly across Pipeline restarts (verify by computing the same indicator on a replayed stream)
- [ ] `yata` and `ta-lib` backends produce identical output (within float epsilon) for `MACD` / `EMA` / `RSI`
- [ ] Plugin README documents: registered UDF signatures, state storage location, backend choice rationale, performance characteristics

---

### S39 · `bee-plugin-onnx-ml`: production-grade ONNX ML model Handlers (real `tract` runtime + FinBERT)

- **Type**: AFK
- **Blocked by**: S00, S05, S15
- **ADRs**: 0005, 0009, 0010

**Crate**: `plugins/bee-plugin-onnx-ml/` (independent `cdylib`)

> **Note**: this is a **Handler** plugin. No Datasource config; the plugin registers SQL UDFs that wrap ONNX models loaded from disk.

**Plugin config (plugin-level, not Datasource)**

```jsonc
{
  "sentiment_model_path": "./models/finbert-quant.onnx",   // ProsusAI FinBERT, fine-tuned for financial sentiment
  "decision_model_path":  "./models/btc-direction-1h.onnx", // Real model, user-supplied (e.g., a gradient-boosted tree exported to ONNX)
  "max_batch_size":       32,
  "device":               "cpu"   // "cpu" | "gpu" (1.x); MVP is CPU-only
}
```

**Handler contract (real `tract` ONNX runtime)**

| Function | Signature | Model | Use case |
| --- | --- | --- | --- |
| `sentiment_score(text_col)` | SQL UDF | FinBERT (ProsusAI, ONNX) | Returns a float in `[-1, 1]`: negative = bearish, positive = bullish |
| `sentiment_class(text_col)` | SQL UDF | FinBERT (ProsusAI, ONNX) | Returns one of `{"positive", "neutral", "negative"}` |
| `price_direction(features_struct)` | SQL UDF | User-supplied ONNX model | Returns one of `{"up", "down", "flat"}` for the next bar |
| `model_score(model_name, features_struct)` | SQL UDF | Generic | Returns the model's raw output (float or class index) |

**Model loading**

- Models are loaded **once at plugin init**; their session lives in plugin-managed memory
- The model file path is part of plugin config (not Datasource config) because the model is a binary artifact, not a connection
- `tract` is pure Rust — no C++ runtime, no `libtorch` dependency

**Batching**

- `sentiment_score` accepts one text per call, but the plugin batches up to `max_batch_size` calls into a single `tract` inference to amortize overhead
- This is transparent to the SQL user

**Acceptance criteria**

- [ ] Independent `cdylib` crate, only depends on `bee-plugin-sdk`, `tract-onnx`, `ndarray`, `tokenizers` (for FinBERT's WordPiece), `serde`, `bincode`
- [ ] `bee plugin load` succeeds; UDFs appear in `bee dsl functions` list
- [ ] `sentiment_score("Bitcoin surges past $100k as institutional demand grows")` returns a positive float in `[0.5, 1.0]` (verified against FinBERT reference output)
- [ ] `sentiment_score("BTC plunges 20% amid regulatory crackdown")` returns a negative float in `[-1.0, -0.5]`
- [ ] Batching: a 100-row burst of `sentiment_score` calls completes in ≤ 10 model invocations (verifiable via debug log)
- [ ] Decision model: `price_direction(struct_pack(ema26, rsi14, macd, sentiment))` returns the right class for a held-out test set (user provides the test)
- [ ] No model weights bundled in the plugin crate; models are loaded from `plugin_config.model_path` at runtime
- [ ] Plugin README documents: registered UDF signatures, model file format (ONNX), batching behavior, expected model input/output schemas, performance characteristics (CPU inference latency)

---

### S40 · Production end-to-end deploy: `examples/quant_btc_strategy.sql` + demo script

- **Type**: AFK
- **Blocked by**: S34, S35, S36, S37, S38, S39, S17, S20
- **ADRs**: 0001, 0003, 0005, 0006, 0009, 0010, 0011

**What this delivers**: the running S33 milestone. Six production plugins loaded, two SQL pipelines deployed, Producer sharing verified, failover verified, real money signals flowing.

**Deliverables**

#### 1. The canonical SQL Pipeline: `examples/quant_btc_strategy.sql`

```sql
use binance;
use google_news;
use influxdb;
use mongodb;

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
```

#### 2. The backfill variant: `examples/quant_btc_strategy_backfill.sql`

Same `use` declarations and the same downstream views, but the binance call is:

```sql
FROM binance.subscribe('BTC/USDT', '5min', from => '2024-06-01');
```

This triggers the S34 backfill path: the Producer first emits historical K-lines from 2024-06-01 to the high-water mark, then continues with live WS. Used for the "warm up the state" step at deploy time.

#### 3. The second strategy, `examples/quant_btc_strategy_v2.sql`

Same `use` declarations, different filter / decision logic. Demonstrates that the same `binance` Datasource (and the same `binance.subscribe('BTC/USDT','5min')` Stream) is shared between two strategies — only one `binance` Producer in the cluster.

#### 4. One-click demo script: `scripts/demo-quant-prod.sh`

Idempotent end-to-end runner. **Requires the user to supply real credentials** via env vars or a `.env` file (NOT checked into the repo):

```bash
#!/usr/bin/env bash
set -euo pipefail

# 0. User must supply credentials (see scripts/.env.example)
[ -f scripts/.env ] || { echo "Missing scripts/.env — see scripts/.env.example"; exit 1; }
. scripts/.env

# 1. Build all 6 production plugins
for plugin in plugins/bee-plugin-{binance,google-news,influxdb,mongodb,ta-indicators,onnx-ml}; do
  (cd "$plugin" && cargo build --release)
done

# 2. Drop all plugins into the plugin dir
mkdir -p /tmp/bee_prod_plugins
cp plugins/bee-plugin-*/target/release/libbee_plugin_*.{so,dylib} /tmp/bee_prod_plugins/

# 3. Start 3-node cluster (delegated to scripts/start-cluster.sh)
scripts/start-cluster.sh

# 4. Register the 4 Datasources (Providers) — connection-level config only
bee datasource create binance \
  --adapter binance_subscribe \
  --plugin-id "$(sha256sum plugins/bee-plugin-binance/target/release/libbee_plugin_binance.so | cut -d' ' -f1)" \
  --config "$(jq -n --arg k "$BINANCE_API_KEY" '{ws_url:"wss://stream.binance.com:9443",rest_url:"https://api.binance.com",api_key:$k,rate_limit_per_sec:10}')"

bee datasource create google_news \
  --adapter google_news_search \
  --plugin-id "$(sha256sum plugins/bee-plugin-google-news/target/release/libbee_plugin_google_news.so | cut -d' ' -f1)" \
  --config "$(jq -n --arg k "$NEWSAPI_KEY" '{base_url:"https://newsapi.org/v2",api_key:$k,rate_limit_per_sec:5,language:"en"}')"

bee datasource create influxdb \
  --adapter influxdb_write \
  --plugin-id "$(sha256sum plugins/bee-plugin-influxdb/target/release/libbee_plugin_influxdb.so | cut -d' ' -f1)" \
  --config "$(jq -n --arg t "$INFLUXDB_TOKEN" --arg o "$INFLUXDB_ORG" '{url:"http://localhost:8086",token:$t,org:$o,bucket:"trading",timeout_ms:5000}')"

bee datasource create mongodb \
  --adapter mongodb_insert \
  --plugin-id "$(sha256sum plugins/bee-plugin-mongodb/target/release/libbee_plugin_mongodb.so | cut -d' ' -f1)" \
  --config "$(jq -n --arg u "$MONGODB_URI" '{uri:$u,database:"trading",app_name:"bee",tls:false}')"

# 5. Deploy the warmup + main pipeline
bee deploy examples/quant_btc_strategy_backfill.sql  # warm up from 2024-06-01
bee deploy examples/quant_btc_strategy.sql
bee deploy examples/quant_btc_strategy_v2.sql         # shares binance Producer

# 6. Wait for the live signals to flow
sleep 60

# 7. Verify outputs hit the real sinks
echo "==== InfluxDB query ===="
curl -sG "http://localhost:8086/api/v2/query?org=${INFLUXDB_ORG}" \
  --header "Authorization: Token ${INFLUXDB_TOKEN}" \
  --data-urlencode "bucket=trading" \
  --data-urlencode 'q=from(bucket:"trading") |> range(start:-5m) |> filter(fn: (r) => r._measurement == "klines") |> limit(n: 5)'

echo "==== MongoDB query ===="
mongosh --quiet "mongodb://localhost:27017/trading" \
  --eval 'db.trades.find().sort({ts:-1}).limit(3).toArray()'

# 8. Verify Producer sharing
N_PRODUCERS=$(bee jobs list --filter 'producer' | wc -l)
test "$N_PRODUCERS" -eq 1 && echo "✓ Producer sharing OK" || (echo "✗ Expected 1 binance Producer"; exit 1)

# 9. Verify failover: kill the Node hosting the binance Producer; both strategies continue
scripts/kill-node.sh node-1
sleep 30
N_RUNNING=$(bee jobs list --filter 'status=running' | wc -l)
test "$N_RUNNING" -eq 2 && echo "✓ Failover OK" || (echo "✗ Expected both strategies to recover"; exit 1)
```

#### 5. `scripts/.env.example`

Documents the required user-supplied env vars:

```
# scripts/.env — copy to scripts/.env and fill in real values; never commit
BINANCE_API_KEY=...           # optional for public market data
NEWSAPI_KEY=...               # required; from https://newsapi.org
INFLUXDB_URL=http://localhost:8086
INFLUXDB_TOKEN=...            # required
INFLUXDB_ORG=...              # required
MONGODB_URI=mongodb://localhost:27017
```

#### 6. README.md and product-design.md updates

- README.md "Quickstart" section now reads: "See [`scripts/demo-quant-prod.sh`](scripts/demo-quant-prod.sh) for a production-grade end-to-end walkthrough. You'll need to supply credentials in `scripts/.env` first."
- The canonical example SQL files land at `docs/best-practices/quant/examples/quant_btc_strategy*.sql` (replacing the current `quant_btc_macd.sql` + `quant_btc_sentiment.sql` from the 5-mock-plugin demo), with the 6 prod plugins.

**Acceptance criteria**

- [ ] All 6 production plugin crates build independently via `cargo build --release`
- [ ] Each plugin's `.so`/`.dylib` is a separate file; one plugin's failure does not block the others
- [ ] `bee plugin list` shows all 6 plugins with distinct `PluginId` (sha256 hashes) and their declared `abi_version`
- [ ] All 4 Datasource registrations via `bee datasource create` succeed; the configs contain **only** connection-level fields (no `symbol`, no `interval`, no `collection`, no `measurement`, no `query`)
- [ ] `bee compile examples/quant_btc_strategy.sql` passes (0 errors, 0 warnings) — strict-mode `use` enforcement validated; `symbol`/`interval`/`measurement`/`collection` are per-call args
- [ ] `bee compile examples/quant_btc_strategy_backfill.sql` passes; the `from => '2024-06-01'` arg is accepted
- [ ] `bee deploy examples/quant_btc_strategy.sql` deploys a Job that produces events to the real InfluxDB and real MongoDB
- [ ] `bee deploy examples/quant_btc_strategy_v2.sql` deploys a second Job; `bee jobs list` shows **both Jobs reference the same `binance` Datasource but have separate Streams**; the `binance` Producer count is exactly 1 (StreamSignature sharing)
- [ ] The backfill variant (`quant_btc_strategy_backfill.sql`) actually emits historical K-lines from 2024-06-01 to the HWM, then seamlessly transitions to live WS (verified by ordered timestamps at the Subscriber)
- [ ] Killing the Node that hosts the `binance` Producer triggers Work-Stealing; both strategies continue within 1 Orphaned period (≤ 30s)
- [ ] After all ADRs' "Consequences" sections, run the demo and **explicitly check** each one:
  - [ ] ADR-0001: data still flows P2P; control still goes through Raft
  - [ ] ADR-0002: Datasource Phase appears in DAG with `adapter` field
  - [ ] ADR-0003: shared Stream serves both strategies
  - [ ] ADR-0004: Task state / checkpoints visible in KV (`bee kv get state/...`); backfill state is visible too
  - [ ] ADR-0005: plugins are `cdylib`; ABI check passes
  - [ ] ADR-0006: SQL extensions (`ASOF JOIN`, `EMIT INTO`, UDFs) work
  - [ ] ADR-0007: cluster runs in simplified all-in-one topology
  - [ ] ADR-0008: scheduler policy observable (`bee cluster status`)
  - [ ] ADR-0009: dropping a new version of a plugin (e.g., `binance v2`) loads alongside v1; `bee plugin list` shows both
  - [ ] ADR-0010: `use` syntax enforced; per-call args go in SQL; Provider / Stream separation works; `collection` is per-call for mongodb
  - [ ] ADR-0011: Stream identity scope; backfill-on-subscribe; per-Subscriber offsets
- [ ] README.md Quickstart links to `scripts/demo-quant-prod.sh`
- [ ] `docs/best-practices/quant/README.md` (and `docs/product-design.md` if a Quant scenario is re-added) references `examples/quant_btc_strategy.sql` and the 6 prod plugins
- [ ] **S33 HITL review done**: first seed user walkthrough; feedback captured; gaps recorded as new stories or ADR amendments

---
