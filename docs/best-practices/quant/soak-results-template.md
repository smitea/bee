# S33.2 24h soak results — [fill in date + run_id]

**Run ID**: [from `/tmp/bee_soak_<RUN_ID>_tick_*.json`]
**Start time** (UTC): [from `ts_unix_ms` of tick 1]
**End time** (UTC): [from `ts_unix_ms` of last tick]
**Operator**: [name]
**Total ticks**: [count of JSON files in `/tmp/bee_soak_<RUN_ID>_tick_*.json`]
**Failover injected**: [Y/N]

## Per-hour summary

| Hour | elapsed (h) | influx klines | mongo trades | node1 role | node2 role | node3 role | any orphan | any lag spike |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0-1 | | | | | | | | |
| 1-2 | | | | | | | | |
| ... | | | | | | | | |
| 23-24 | | | | | | | | |

(Fill in by reading the tick JSON files. The `influx klines` and `mongo trades` columns are the cumulative `*_per_min` × 60 for that hour.)

## Failover section (if injected)

- Kill time (UTC): [from `failover_at_ms`]
- Recovered time (UTC): [from `recovered_at_ms`]
- Transition time: [diff in seconds]
- Post-recovery observations: [free text — any oddities in the per-hour table around the transition hour?]

## Threshold breaches

- (a) Log lag > 1000 entries: [Y/N — if Y, list the offending ticks]
- (b) Task Orphaned > 60s without Work-Stealing: [Y/N]
- (c) InfluxDB / MongoDB rate == 0 for ≥ 10 min: [Y/N — if Y, list the affected window]

## Summary

- Total InfluxDB rows observed: [sum of `*_per_min` × `interval_sec`/60 across all ticks]
- Total MongoDB rows observed: [same]
- Total decisions: [N/A for MVP — TaskRuntimeStats / messages_processed sum]
- Total errors: [N/A for MVP — same]
- Total uptime %: [100 if no failover threshold breach; otherwise compute (total_sec - outage_sec) / total_sec]

## Verdict

- [ ] PASS — all thresholds OK, soak completes cleanly
- [ ] FAIL — one or more thresholds breached; see above

## Sign-off

- Operator: ___________________________ Date: __________
- S33 sign-off rows this run enables:
  - [ ] Real money signals observed
  - [ ] InfluxDB data verified
  - [ ] MongoDB data verified
  - [ ] Failover verified (if --failover-midway was used)

## Raw ticks

- `/tmp/bee_soak_<RUN_ID>_tick_*.json` — one JSON file per 5-min interval
- `bee --connect $LEADER kv list soak/run_<RUN_ID>/` — Raft-KV copy of the same metrics
  (only present if the cluster's AdminServer writes via kv put; Task 9 ships
  the JSON file path; the kv put wiring is a follow-up.)
