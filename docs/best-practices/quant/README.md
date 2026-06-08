# Bee · Best Practices · Quant Trading

This section is the **quant trading reference implementation** for
Bee — a large chunk of real-world business example that exercises
every Bee feature end-to-end (Datasource management, Producer
sharing, plugin loading, FFI dispatch, SQL pipelines, deployment).

## What's here

- `stories.md` — the quant-trading implementation stories
  (S33 HITL milestone + S34–S40 production plugins + e2e
  deploy). Cross-references to other stories point back to
  the main repo's `docs/stories.md`.
- `adr/0011-stream-identity-and-backfill.md` — the
  Stream-identity ADR. Quant-specific (covers the Binance WS
  backfill-on-subscribe semantics).
- `examples/quant_btc_macd.sql` + `quant_btc_sentiment.sql` —
  two end-to-end SQL pipelines: BTC K-line + MACD/EMA (technical
  only) and BTC K-line + FinBERT sentiment + decision tree
  (technical + ML).
- `scripts/demo-quant-prod.sh` — architecture-level smoke demo
  for the FFI + runtime dispatching + 5 plugins + 2 pipelines.
- `specs/2026-06-08-s33-deferred-ffi-design.md` — the design
  spec for the FFI wire format + runtime plugin dispatching.
- `plans/2026-06-08-s33-deferred-ffi.md` — the implementation
  plan for the FFI + runtime dispatching.

## Why a separate section

The main repo's primary story is the **generic, domain-agnostic
Bee** — Producer sharing, plugin FFI, performance showcase (S41).
The quant trading example is too large and too domain-specific to
be the primary narrative; it's preserved here as a reference for
users who want to build real quant strategies on top of Bee.

The 5 plugins under `plugins/quant/` are *reference
implementations* — their plugin STRUCTURE is production-grade
(cdylib + FFI vtable + bincode wire format), but the data
sources (Binance WS, NewsAPI, InfluxDB v2, MongoDB, yata/ta-lib)
are placeholders. S34–S40 in `stories.md` replace them with real
implementations.

## See also

- Main repo `docs/stories.md` for the generic Bee story set
  (S0–S31, S41).
- Main repo `docs/adr/0001`–`0010` for the generic architecture
  decisions.
- Main repo `plugins/` for the S41 performance plugins (land in
  a future session).
