# Bee S41 Performance Demos

This directory contains the three SQL pipelines that ship as the S41
performance showcase (1-node MVP). Each file is a standalone `.sql`
script that runs end-to-end via `bee run`.

## Demos

| File | Concept | What it exercises |
|------|---------|-------------------|
| `fibonacci.sql` | Stateful Handler UDF + KV-backed state | Smallest possible streaming-compute surface |
| `prime_sieve.sql` | Sequential sieving Phases (Eratosthenes) | Composability + hard correctness check (`n_primes = 5761455`) |
| `multi_stream_analytics.sql` | Multiple streams joined by time | ASOF JOIN extension (Bee translator) + aggregation |

## Running

Each demo runs standalone:

```bash
cargo run -p bee --bin bee -- run examples/performance/<file>.sql
```

The 3-demo runner script (`scripts/demo-perf.sh`, Task 13) wraps all
three and prints a measured wall-clock table.
