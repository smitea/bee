# ADR-0011 · Stream identity scope & backfill-on-subscribe semantics

- **Status**: Accepted
- **Date**: 2026-06-07
- **Supersedes / refines**: ADR-0003 (Producer Pipeline pattern) and ADR-0010 (Datasource as managed Provider)
- **Author**: Bee core team

## Context

ADR-0010 establishes that a Datasource (Provider) config holds **only** connection-level parameters (credentials, base URL, rate limits), while per-call arguments (symbol, interval, query string) live in the SQL call site. ADR-0003 establishes that multiple Pipelines can share a single Producer.

This raises two questions the prior ADRs do not answer concretely:

1. **What exactly identifies a Stream?** A Stream must have a stable identity so that the runtime can decide whether two SQL calls share a Producer. ADR-0010 says "symbol, interval go in the call site" but does not say whether they are part of the Stream identity or merely a per-call concern.
2. **How does a new Subscriber resume from historical data?** The Stream is live; if a Subscriber wants to start from a past timestamp, what mechanism does the runtime provide? Naive answers (e.g. "Stream identity includes the `from` argument") would force a separate Producer per backfill range, defeating the sharing principle.

Without precise answers, plugin authors would invent their own conventions, and cross-plugin behavior would diverge. S34 (the production binance plugin) needs these answers before it can be implemented.

## Decision

### 1. Stream identity scope

`StreamSignature` is the hash of the **Stream topology**, not the **per-call resumption parameters**:

```
StreamSignature = sha256(
    datasource_name ||
    adapter_method   ||
    hash(stream_topology_args)
)
```

where `stream_topology_args` is the set of arguments that, if changed, would change the **identity of the underlying external system resource**.

For example, for `binance.subscribe(...)`:

```
StreamSignature = sha256("binance" || "subscribe" || sha256(symbol || interval))
```

`symbol` and `interval` are part of the Stream identity because they identify the external WebSocket subscription (`<symbol>@kline_<interval>`). Two Pipelines calling `binance.subscribe('BTC/USDT', '5min')` share one Producer; one calling `binance.subscribe('ETH/USDT', '5min')` requires a different Producer.

The `from` argument is **not** in the signature — it is a per-Subscriber resumption parameter (see below).

### 2. Backfill-on-subscribe semantics

When a Subscriber calls `subscribe(topology_args, from?)` where `from < now`, the plugin is responsible for:

1. Reading the Producer's high-water mark `H` from KV (`state/producer/<stream_id>/hwm`)
2. If `from < H`: internally calling its `download_history(topology_args, from, H)` method and emitting the historical events in time order
3. If `from >= H` or `from` is null: skipping backfill
4. Then opening / resuming the live subscription (WS, polling, change-stream, etc.)
5. Emitting both historical and live events on the **same Stream** with a monotonically increasing offset

The Subscriber's own Task State stores the last-consumed offset. On Subscriber restart (not Producer restart), the Subscriber asks for backfill from its own last offset — independent of the Producer's HWM and independent of any other Subscriber's offset.

This is the same model as Kafka: a topic identity does not include consumer offsets; consumers seek to their own offsets within a shared topic.

### 3. The `download_history` adapter method

For Adapters that back historical data with a REST API (Binance, NewsAPI search, InfluxDB query, etc.), the Adapter contract **must** expose a `download_history` (or equivalent) method as part of its public API, not just an internal helper. This is so:

- Other Adapters (e.g. an `EMIT INTO` sink that wants to bootstrap from history) can call it
- The runtime can implement generic "replay from timestamp X" commands without per-plugin code
- Testing the backfill path is independent of testing the live path

For Adapters that have no historical equivalent (e.g. webhook inputs), the method returns `Unsupported` and the runtime degrades gracefully (no backfill available; live-only).

### 4. Per-call collection / measurement / query

Arguments that route to **different external system resources of the same kind** are part of the Stream identity:

- `mongodb.insert(collection, doc)`: `collection` is in the signature (different collection = different Stream identity)
- `influxdb.write(measurement, ...)`: `measurement` is in the signature for Input (`query`) direction; Output direction does not produce a Stream
- `google_news.search(query, ...)`: `query` is in the signature

Arguments that are **purely consumer-side** (filters, projections, time windows) are not in the signature.

The full set of "identity" vs "per-call" arguments is documented per plugin in S34–S39.

## Consequences

### Positive

- Producer sharing is maximized: every change to a *per-call* (consumer-side) parameter does not spawn a new Producer; only changes to the *external resource identity* do
- Backfill is a uniform runtime concept: the runtime can answer "what's the offset at time T?" via a single protocol
- Plugin authors have a clear contract: which Adapter method exposes `download_history`, which arguments are identity vs per-call
- Multiple Subscribers with different resumption points share one Producer and one WS connection (or REST polling cycle, or change-stream cursor)

### Negative

- The Plugin SDK must expose `download_history` as a first-class concept; the SDK's API surface grows
- Backfill adds a non-trivial state machine to the Producer (it must coordinate "fetching history" and "streaming live" without emitting duplicates or gaps)
- Per-Subscriber offsets mean the runtime must store offset state per Subscriber, not just per Producer

### Neutral

- ADR-0003's "StreamSignature" concept gets a more precise definition; the spirit is unchanged
- ADR-0010's "per-call args in SQL" principle is preserved; this ADR clarifies which per-call args are identity vs consumer-side

## Validation

Validated by **S40** (production end-to-end deploy) in `docs/stories.md`:

- Two Pipelines calling `binance.subscribe('BTC/USDT', '5min')` share one Producer (no extra Producer spawned)
- One Pipeline calling `binance.subscribe('BTC/USDT', '5min', from => '2024-06-01')` triggers a backfill; the same Pipeline without `from` does not
- A Subscriber that is restarted mid-stream resumes from its own last offset (not from the Producer's HWM, not from epoch)
- The `mongodb` and `google_news` plugins follow the same identity rules
