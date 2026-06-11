#!/usr/bin/env bash
# scripts/soak-quant-24h.sh — S33.2: 24h live-soak
# monitoring loop for the quant production plugins.
#
# Usage:
#   bash scripts/soak-quant-24h.sh [--smoke]
#                                   [--failover-midway]
#                                   [--interval-secs N]
#                                   [--run-id ID]
#                                   [--node 2]
#
# Defaults: --interval-secs 300 (5 min), 24h total.
# --smoke overrides: --interval-secs 5, 5 min total
# (60 ticks; CI-friendly).
# --failover-midway injects a SIGKILL at the
# 12h mark (or 2.5 min for --smoke).
#
# Exit codes:
#   0   clean run, all thresholds OK
#   1   any threshold triggered
#   2   bad flags
#   3   bootstrap failed (deploy did not produce
#       data within 5 min)
#   130 human Ctrl-C
#
# See docs/superpowers/specs/2026-06-10-s33-2-24h-live-soak-design.md
# for the full design.

set -euo pipefail
cd "$(dirname "$0")/.."

# 1. Flags
SMOKE=0
FAILOVER_MIDWAY=0
INTERVAL_SECS=300
TOTAL_SECS=$((24 * 3600))
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
KILL_NODE=2
while [ $# -gt 0 ]; do
    case "$1" in
        --smoke) SMOKE=1; shift ;;
        --failover-midway) FAILOVER_MIDWAY=1; shift ;;
        --interval-secs) INTERVAL_SECS="$2"; shift 2 ;;
        --run-id) RUN_ID="$2"; shift 2 ;;
        --node) KILL_NODE="$2"; shift 2 ;;
        *) echo "unknown flag $1" >&2; exit 2 ;;
    esac
done
if [ "$SMOKE" = "1" ]; then
    INTERVAL_SECS=5
    TOTAL_SECS=300
fi

# 2. .env check
if [ ! -f scripts/.env ]; then
    echo "ERROR: scripts/.env is missing." >&2
    echo "  cp scripts/.env.example scripts/.env" >&2
    echo "  then fill in NEWSAPI_KEY, INFLUXDB_TOKEN, INFLUXDB_ORG." >&2
    exit 2
fi
# shellcheck disable=SC1091
. scripts/.env
: "${INFLUXDB_URL:?INFLUXDB_URL is required}"
: "${INFLUXDB_TOKEN:?INFLUXDB_TOKEN is required}"
: "${INFLUXDB_ORG:?INFLUXDB_ORG is required}"
: "${MONGODB_URI:?MONGODB_URI is required}"

BEE=./target/debug/bee
[ -x "$BEE" ] || {
    echo "building bee..." >&2
    cargo build --quiet -p bee
}

# 3. Trap for cleanup
cleanup() {
    echo "trap: cleaning up..."
    for pid in $(awk '{print $2}' /tmp/bee_cluster.pids 2>/dev/null); do
        kill -9 "$pid" 2>/dev/null || true
    done
    rm -f /tmp/bee_cluster.pids
}
trap cleanup EXIT

START_MS=$(date +%s%3N)
FAILOVER_AT_MS=""
RECOVERED_AT_MS=""

# 4. Phase 0: build plugins (skip for --smoke; the
# start-cluster.sh script handles building the bee
# binary if missing, but plugin cdylibs are needed
# for datasource registration).
if [ "$SMOKE" = "0" ]; then
    if [ -x scripts/build-prod-plugins.sh ]; then
        bash scripts/build-prod-plugins.sh 2>/dev/null || {
            echo "WARNING: build-prod-plugins.sh failed; assuming plugins already built"
        }
    fi
fi

# 5. Phase 1: start 3-node cluster
echo "phase 1: starting 3-node cluster..."
scripts/start-cluster.sh --nodes 3

# 6. Phase 2: discover leader
echo "phase 2: discovering leader..."
LEADER_ADDR=""
for i in $(seq 1 30); do
    for n in 1 2 3; do
        ADDR="127.0.0.1:$((8700 + n))"
        if OUT=$("$BEE" --connect "$ADDR" cluster status 2>/dev/null); then
            if echo "$OUT" | grep -q "leader=Some"; then
                LEADER_ADDR="$ADDR"
                break 2
            fi
        fi
    done
    sleep 1
done
if [ -z "$LEADER_ADDR" ]; then
    echo "ERROR: no leader discovered within 30s" >&2
    exit 1
fi
echo "  leader: $LEADER_ADDR"

# 7. Phase 3 + 4: register datasources + deploy pipelines.
# S33.3 wired these via the admin RPC CLI; the
# AdminServer on the leader writes a 'marker' to
# its local KV (S33.3 MVP — see admin_server.rs).
# The full bee-dsl-sql runner is a S33.4 follow-up.
echo "phase 3: registering datasources..."
for ds in binance google_news influxdb mongodb; do
    case "$ds" in
        binance) ADAPTER=binance ;;
        google_news) ADAPTER=google_news ;;
        influxdb) ADAPTER=influxdb ;;
        mongodb) ADAPTER=mongodb ;;
    esac
    if "$BEE" --connect "$LEADER_ADDR" datasource create "$ds" \
        --adapter "$ADAPTER" \
        --plugin-version 1.0.0 \
        --config '{}' 2>/dev/null; then
        echo "  $ds registered (KV marker)"
    else
        echo "  WARN: $ds create via admin RPC failed (cluster might still be electing)"
    fi
done

echo "phase 4: deploying pipelines..."
for sql in docs/best-practices/quant/examples/quant_btc_strategy_backfill.sql \
          docs/best-practices/quant/examples/quant_btc_strategy.sql \
          docs/best-practices/quant/examples/quant_btc_strategy_v2.sql; do
    if [ -f "$sql" ]; then
        if "$BEE" --connect "$LEADER_ADDR" deploy "$sql" 2>/dev/null; then
            echo "  $sql deployed (KV marker)"
        else
            echo "  WARN: deploy $sql via admin RPC failed"
        fi
    else
        echo "  WARN: $sql not found; skipping"
    fi
done

# 8. Phase 5: bootstrap check (5 min wait for first data)
echo "phase 5: bootstrap check (5 min wait)..."
BOOTSTRAP_OK=0
for i in $(seq 1 10); do
    sleep 30
    KLINES=$(curl -sG "$INFLUXDB_URL/api/v2/query?org=$INFLUXDB_ORG" \
        --header "Authorization: Token $INFLUXDB_TOKEN" \
        --data-urlencode "bucket=trading" \
        --data-urlencode 'q=from(bucket:"trading") |> range(start:-5m) |> filter(fn: (r) => r._measurement == "klines") |> count()' 2>/dev/null | wc -l | tr -d ' ')
    if [ "$KLINES" -gt 1 ]; then
        BOOTSTRAP_OK=1
        break
    fi
done
if [ "$BOOTSTRAP_OK" = "0" ]; then
    echo "ERROR: no influxdb klines within 5 min" >&2
    exit 3
fi
echo "  bootstrap OK (klines present)"

# 9. Phase 6: human gate
echo "phase 6: bootstrap OK. Hit Enter to start the ${TOTAL_SECS}s loop, or Ctrl-C to abort."
read -r _ || true

# 10. Phase 7 + 8: monitoring loop
echo "phase 7: monitoring loop (interval=${INTERVAL_SECS}s, total=${TOTAL_SECS}s)..."
TICK=0
INFLUX_ZERO_TICKS=0
MONGO_ZERO_TICKS=0
mkdir -p /tmp/bee_soak
while :; do
    TICK=$((TICK + 1))
    NOW_MS=$(date +%s%3N)
    ELAPSED_SEC=$(( (NOW_MS - START_MS) / 1000 ))
    if [ "$ELAPSED_SEC" -ge "$TOTAL_SECS" ]; then
        echo "  total elapsed, exiting loop"
        break
    fi

    # 10a. Failover injection (T+half of TOTAL_SECS)
    if [ "$FAILOVER_MIDWAY" = "1" ] && [ -z "$FAILOVER_AT_MS" ]; then
        HALF=$((TOTAL_SECS / 2))
        if [ "$ELAPSED_SEC" -ge "$HALF" ]; then
            echo "  failover injection: killing node $KILL_NODE"
            scripts/kill-node.sh --node "$KILL_NODE" || true
            FAILOVER_AT_MS="$NOW_MS"
        fi
    fi

    # 10b. Re-discover leader if we just failovered
    if [ -n "$FAILOVER_AT_MS" ] && [ -z "$RECOVERED_AT_MS" ]; then
        for n in 1 2 3; do
            ADDR="127.0.0.1:$((8700 + n))"
            if "$BEE" --connect "$ADDR" cluster status >/dev/null 2>&1; then
                if [ "$ADDR" != "$LEADER_ADDR" ]; then
                    LEADER_ADDR="$ADDR"
                    RECOVERED_AT_MS="$NOW_MS"
                    echo "  failover recovered, new leader: $LEADER_ADDR"
                fi
                break
            fi
        done
    fi

    # 10c. Per-tick metrics
    CLUSTER_JSON=$("$BEE" --connect "$LEADER_ADDR" cluster status 2>/dev/null || echo "ERROR")
    JOBS_JSON=$("$BEE" --connect "$LEADER_ADDR" jobs 2>/dev/null || echo "ERROR")
    KLINES_PER_MIN=$(curl -sG "$INFLUXDB_URL/api/v2/query?org=$INFLUXDB_ORG" \
        --header "Authorization: Token $INFLUXDB_TOKEN" \
        --data-urlencode "bucket=trading" \
        --data-urlencode 'q=from(bucket:"trading") |> range(start:-5m) |> filter(fn: (r) => r._measurement == "klines") |> count()' 2>/dev/null | grep -c "^_" || true)
    if [ -z "$KLINES_PER_MIN" ]; then KLINES_PER_MIN=0; fi
    TRADES_PER_MIN=0
    if command -v mongosh >/dev/null 2>&1; then
        TRADES_PER_MIN=$(mongosh --quiet "$MONGODB_URI/trading" --eval 'db.trades.countDocuments({ts: {$gte: new Date(Date.now() - 5*60*1000)}}).toString()' 2>/dev/null || echo "0")
    fi

    # 10d. Threshold checks
    THRESHOLD_FAIL=0
    if [ "$KLINES_PER_MIN" = "0" ]; then
        INFLUX_ZERO_TICKS=$((INFLUX_ZERO_TICKS + 1))
        # 10 min of zero rate = 2 default-ticks or 120 --smoke-ticks
        ZERO_LIMIT=$(( 10 * 60 / INTERVAL_SECS ))
        if [ "$INFLUX_ZERO_TICKS" -ge "$ZERO_LIMIT" ]; then
            echo "  THRESHOLD: influxdb rate == 0 for $((INFLUX_ZERO_TICKS * INTERVAL_SECS))s"
            THRESHOLD_FAIL=1
        fi
    else
        INFLUX_ZERO_TICKS=0
    fi
    if [ "$TRADES_PER_MIN" = "0" ]; then
        MONGO_ZERO_TICKS=$((MONGO_ZERO_TICKS + 1))
        ZERO_LIMIT=$(( 10 * 60 / INTERVAL_SECS ))
        if [ "$MONGO_ZERO_TICKS" -ge "$ZERO_LIMIT" ]; then
            echo "  THRESHOLD: mongodb rate == 0 for $((MONGO_ZERO_TICKS * INTERVAL_SECS))s"
            THRESHOLD_FAIL=1
        fi
    else
        MONGO_ZERO_TICKS=0
    fi
    if [ "$THRESHOLD_FAIL" = "1" ]; then
        exit 1
    fi

    # 10e. Persist tick as JSON
    TICK_FILE="/tmp/bee_soak/${RUN_ID}_tick_${NOW_MS}.json"
    cat > "$TICK_FILE" <<EOF
{
  "ts_unix_ms": $NOW_MS,
  "elapsed_sec": $ELAPSED_SEC,
  "cluster": $(echo "$CLUSTER_JSON" | jq -R -s '.' 2>/dev/null || echo '"ERROR"'),
  "jobs": $(echo "$JOBS_JSON" | jq -R -s '.' 2>/dev/null || echo '"ERROR"'),
  "influx_klines_per_min": $KLINES_PER_MIN,
  "mongo_trades_per_min": $TRADES_PER_MIN,
  "failover_at_ms": $([ -n "$FAILOVER_AT_MS" ] && echo "$FAILOVER_AT_MS" || echo "null"),
  "recovered_at_ms": $([ -n "$RECOVERED_AT_MS" ] && echo "$RECOVERED_AT_MS" || echo "null")
}
EOF

    # 10f. Also write to the leader's Raft KV
    # via the admin RPC. The value is the JSON
    # body; the human can read back via
    # `bee --connect <leader> kv list
    # soak/run_<id>/` (S33.3 Task 5). The kv
    # put is best-effort; if the leader's admin
    # RPC is down (e.g. mid-failover), the
    # next tick will retry.
    "$BEE" --connect "$LEADER_ADDR" \
        kv put "soak/${RUN_ID}/tick_${NOW_MS}" \
        "$TICK_FILE" 2>/dev/null || true

    echo "  tick $TICK (T+${ELAPSED_SEC}s): klines=$KLINES_PER_MIN trades=$TRADES_PER_MIN"
    sleep "$INTERVAL_SECS"
done

# 11. Phase 9: summary
echo "phase 9: summary"
echo "  run_id: $RUN_ID"
echo "  ticks: $TICK"
echo "  failover_at_ms: ${FAILOVER_AT_MS:-n/a}"
echo "  recovered_at_ms: ${RECOVERED_AT_MS:-n/a}"
echo "  ticks stored in: /tmp/bee_soak/${RUN_ID}_tick_*.json"
exit 0
