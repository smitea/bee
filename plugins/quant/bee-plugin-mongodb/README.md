# bee-plugin-mongodb

Production-grade MongoDB adapter for Bee (S37 in the quant-trading
reference implementation). Implements real mongodb driver (BSON
encode/decode) + insert/find/update/aggregate operations + per-call
collection override + connection pooling, per
[S37 in the quant stories](../../../../docs/best-practices/quant/stories.md#s37--bee-plugin-mongodb-production-grade-mongodb-adapter-real-driver-per-call-collection).

## Quick start

1. Build the plugin:
   ```bash
   cargo build --release -p bee-plugin-mongodb
   ```
   Output: `target/release/libbee_plugin_mongodb.dylib` (macOS), `.so` (Linux), or `.dll` (Windows).

2. Load it into a running Bee node:
   ```bash
   bee plugin load target/release/libbee_plugin_mongodb.dylib
   bee plugin list  # verify the plugin is listed
   ```

3. Create a Datasource with the connection-level config (see `.env.example`):
   ```bash
   bee datasource create mongodb \
     --adapter mongodb_insert \
     --config @mongodb.example.json
   ```
   The config file is the JSON shown in `.env.example`. **Note**: the config has NO `collection` field — collection is per-call.

4. Use it in a SQL pipeline (collection per-call):
   ```sql
   use mongodb;
   
   EMIT INTO mongodb.insert('trades', doc)               -- insert into 'trades' collection
     SELECT * FROM v_btc_decision;
   
   EMIT INTO mongodb.insert('order_decision', doc)      -- same Datasource, different collection
     SELECT * FROM v_final_decision;
   
   SELECT * FROM mongodb.find('news_articles', {category: 'crypto'})  -- poll/change-stream a collection
   ```

## Datasource config (connection-level only — ADR-0010; note: NO `collection` field)

```jsonc
{
  "uri":       "mongodb://localhost:27017",  // admin-supplied
  "database":  "trading",                    // default DB; collection is per-call
  "username":  "<from bee secret store; optional>",
  "password":  "<from bee secret store; optional>",
  "app_name":  "bee",                        // appears in MongoDB logs
  "tls":       false,
  "tenant":    0
}
```

See `.env.example` for the canonical template.

## Per-call args (in SQL — `collection` is per-call, NOT in Datasource config)

| Method | Args |
| --- | --- |
| `mongodb.insert(collection, document)` | insert one document |
| `mongodb.insert_many(collection, documents)` | batched insert |
| `mongodb.find(collection, filter)` | poll the collection; emit matching docs |
| `mongodb.update(collection, filter, update)` | update one document |
| `mongodb.aggregate(collection, pipeline)` | run an aggregation pipeline; emit result rows |

- `collection` (e.g. `'trades'`, `'order_decision'`, `'news_articles'`) — per-call, by design (ADR-0010)
- For `insert`/`insert_many`: `document` / `documents` (BSON)
- For `find`: `filter` (BSON)
- For `update`: `filter`, `update` (BSON)
- For `aggregate`: `pipeline` (array of BSON stages)

## Why `collection` is per-call (not in Datasource config)

- A single MongoDB cluster holds many collections; the same Datasource `mongodb` should be reusable across all of them.
- Different `use mongodb;` calls with different `collection` args are different Streams (StreamSignature includes collection).
- This matches ADR-0010: **Datasource config = connection-level only; per-call args in SQL**.

## Connection pooling

The plugin holds a single `Arc<mongodb::Client>` shared across all 5 adapters (3 output + 2 input). The official `mongodb` driver handles the connection pool internally; the plugin just borrows the shared client.

A single `mongodb` Datasource can serve all 5 Pipelines simultaneously (different collections, different filters, all sharing the same connection pool).

## Stream identity

- For `find`/`aggregate`: `StreamSignature = sha256("mongodb" || method || database || collection || hash(filter_or_pipeline))` — different filters/pipelines are different Producers.
- For `insert`/`update`: Output adapters don't produce Streams; the signature is `(mongodb, write, database, collection)` — connection-level + collection.

## Change streams

The plugin's `find` method polls the collection at a configurable cadence. For real-time change streams (MongoDB 3.6+ feature), a future 1.x plugin revision will subscribe to the collection's change stream and emit only new/modified documents. For now, polling is used; downstream subscribers must deduplicate.

## Credentials

- The plugin reads `username` / `password` from the Datasource config (which references the Bee secret store).
- 1.x: replace with Vault / AWS Secrets Manager (out of scope).
- **The password is NEVER logged or included in error messages** (per spec).
- The plugin does **not** fall back to environment variables — config is the single source of truth.

## Building

```bash
cargo build --release -p bee-plugin-mongodb
```

## Testing

```bash
cargo test -p bee-plugin-mongodb
```

The unit tests cover:
- `MongodbConfig` default values + bincode round-trip
- `InsertArgs` / `DocumentEvent` / `InsertResult` / `UpdateResult` bincode round-trips
- Stream signatures (`write` / `find` / `aggregate`)
- BSON document round-trip (raw bytes via `bson::to_vec` / `bson::from_slice`)
- Password never appears in error messages
- Per-call collection serialization (different collections → different args bytes)
- Plugin manifest lists 5 adapters
- 20+ unit tests + 1 regression test (for the `DocumentEvent` bincode-bson fix)

Live network tests (against a real MongoDB instance; `docker run mongo:7`) are a follow-up.
