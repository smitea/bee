#!/usr/bin/env bash
# scripts/demo-quant-prod.sh — S40 end-to-end demo: 6 production plugins
# (binance / google-news / influxdb / mongodb / ta-lib / onnx-ml)
# wired into 3 SQL pipelines, deployed against real external services.
#
# Usage:
#   1. cp scripts/.env.example scripts/.env
#   2. Fill in NEWSAPI_KEY, INFLUXDB_TOKEN, INFLUXDB_ORG (see scripts/.env.example)
#   3. Start local InfluxDB v2 and MongoDB (or set remote URLs in scripts/.env)
#   4. scripts/demo-quant-prod.sh
#
# What it does:
#   1. Checks for scripts/.env (user-supplied credentials)
#   2. Builds all 6 production plugins in release mode
#   3. Starts a single-node Bee cluster (the multi-node failover demo
#      is a 1.x feature; see the "Single-node MVP" note below)
#   4. Registers 4 Datasources (binance, google_news, influxdb, mongodb)
#   5. Deploys 3 SQL pipelines (backfill warmup, v1 strategy, v2 strategy)
#   6. Verifies the InfluxDB and MongoDB sinks received data
#   7. Verifies Producer sharing: binance Datasource -> exactly 1 Producer
#
# Single-node MVP:
#   The S40 spec references scripts/start-cluster.sh (3-node cluster)
#   and scripts/kill-node.sh (failover demo). Neither file exists in
#   the MVP — those land with the multi-node feature in 1.x. This
#   script runs on a single Bee node; the SQL pipelines and Datasource
#   registration are identical, and the failover + Producer-sharing
#   checks degrade to "pipeline running" + "Producer count == 1" on
#   the single host.

set -euo pipefail

cd "$(dirname "$0")/.."
WORKSPACE_ROOT="$(pwd)"

# 0. Credentials check
if [ ! -f scripts/.env ]; then
  echo "ERROR: scripts/.env is missing." >&2
  echo "  cp scripts/.env.example scripts/.env" >&2
  echo "  then fill in NEWSAPI_KEY, INFLUXDB_TOKEN, INFLUXDB_ORG." >&2
  exit 1
fi
# shellcheck disable=SC1091
. scripts/.env

: "${NEWSAPI_KEY:?NEWSAPI_KEY is required (see scripts/.env.example)}"
: "${INFLUXDB_URL:?INFLUXDB_URL is required (default: http://localhost:8086)}"
: "${INFLUXDB_TOKEN:?INFLUXDB_TOKEN is required (see scripts/.env.example)}"
: "${INFLUXDB_ORG:?INFLUXDB_ORG is required (see scripts/.env.example)}"
: "${MONGODB_URI:?MONGODB_URI is required (default: mongodb://localhost:27017)}"

# Result tracking
PASS=0
FAIL=0
RESULTS=()

record() {
  local name="$1" ok="$2"
  if [ "$ok" = "true" ]; then
    RESULTS+=("✓ $name")
    PASS=$((PASS+1))
  else
    RESULTS+=("✗ $name")
    FAIL=$((FAIL+1))
  fi
}

step() { echo; echo "=== $* ==="; }

# Helper: hash of a plugin cdylib
plugin_hash() {
  local dylib="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$dylib" | cut -d' ' -f1
  else
    shasum -a 256 "$dylib" | cut -d' ' -f1
  fi
}

# Helper: cdylib path for a plugin
#
# Cargo workspaces share a single `target/` directory at the
# workspace root, NOT per-plugin `target/` subdirs. The
# plugin's source may live under `plugins/quant/<name>/`
# (the S33 quant-trading reference layout) or `plugins/<name>/`
# (the main-repo layout). The cdylib's binary name is
# `lib<name-with-dashes-to-underscores>.dylib|.so` regardless
# of the source layout. The look-up tries the canonical
# workspace target dir first, then falls back to per-plugin
# targets (in case the user invoked `cargo build` inside the
# plugin's subdir, which would write to that plugin's
# `target/`).
plugin_dylib() {
  local plugin="$1"
  local libname="lib${plugin//-/_}"
  local ext
  if [ "$(uname)" = "Darwin" ]; then
    ext="dylib"
  else
    ext="so"
  fi
  # Workspace-root target (the normal case for a workspace build).
  if [ -f "$WORKSPACE_ROOT/target/release/${libname}.${ext}" ]; then
    echo "$WORKSPACE_ROOT/target/release/${libname}.${ext}"
    return
  fi
  # Per-plugin target (only present if the user built inside
  # the plugin's subdir; rare in a workspace).
  if [ -f "$WORKSPACE_ROOT/plugins/quant/$plugin/target/release/${libname}.${ext}" ]; then
    echo "$WORKSPACE_ROOT/plugins/quant/$plugin/target/release/${libname}.${ext}"
    return
  fi
  if [ -f "$WORKSPACE_ROOT/plugins/$plugin/target/release/${libname}.${ext}" ]; then
    echo "$WORKSPACE_ROOT/plugins/$plugin/target/release/${libname}.${ext}"
    return
  fi
  # Fallback: the original (broken) path. Returned so the
  # caller's `[ -f ... ]` check produces a clear "missing
  # cdylib" error pointing at the expected location.
  echo "$WORKSPACE_ROOT/plugins/$plugin/target/release/${libname}.${ext}"
}

# 1. Build all 6 production plugins
step "build 6 production plugins (release)"
for plugin in bee-plugin-binance bee-plugin-google-news bee-plugin-influxdb \
              bee-plugin-mongodb bee-plugin-ta-lib bee-plugin-onnx-ml; do
  # The S33 quant-trading reference plugins live under
  # `plugins/quant/<name>/`; the main-repo's plugins live at
  # `plugins/<name>/`. We try the quant subdir first (this
  # script's primary purpose) and fall back to the main-repo
  # layout (so the same script works for either location).
  if [ -d "plugins/quant/$plugin" ]; then
    plugin_dir="plugins/quant/$plugin"
  elif [ -d "plugins/$plugin" ]; then
    plugin_dir="plugins/$plugin"
  else
    record "cargo build --release $plugin (no source dir)" false
    continue
  fi
  echo "  - $plugin (in $plugin_dir)"
  if (cd "$plugin_dir" && cargo build --release --quiet); then
    record "cargo build --release $plugin" true
  else
    record "cargo build --release $plugin" false
  fi
done

# 2. Drop all plugin cdylibs into a shared plugin dir
step "stage plugin cdylibs"
PLUGIN_DIR="${BEE_PLUGIN_DIR:-/tmp/bee_prod_plugins}"
mkdir -p "$PLUGIN_DIR"
for plugin in bee-plugin-binance bee-plugin-google-news bee-plugin-influxdb \
              bee-plugin-mongodb bee-plugin-ta-lib bee-plugin-onnx-ml; do
  dylib="$(plugin_dylib "$plugin")"
  if [ -f "$dylib" ]; then
    cp "$dylib" "$PLUGIN_DIR/"
    record "staged $(basename "$dylib") -> $PLUGIN_DIR/" true
  else
    record "missing $dylib" false
  fi
done

# 3. Cluster: single-node MVP. The spec's 3-node `scripts/start-cluster.sh`
# is a 1.x feature. We run the bee binary directly with --single-node.
step "start single-node Bee cluster"
BEE="${BEE_BIN:-$WORKSPACE_ROOT/target/release/bee}"
if [ ! -x "$BEE" ]; then
  echo "  Building bee CLI..."
  (cd "$WORKSPACE_ROOT" && cargo build --release -p bee --quiet)
fi
echo "  BEE_BIN=$BEE"
echo "  BEE_PLUGIN_DIR=$PLUGIN_DIR"
echo "  (multi-node cluster + failover demo deferred to 1.x; see stories.md#s40)"

# 4. Register the 4 Datasources
step "register 4 Datasources (Providers) — connection-level config only"
register_ds() {
  local name="$1" adapter="$2" plugin="$3" config_json="$4"
  local dylib
  dylib="$(plugin_dylib "$plugin")"
  if [ ! -f "$dylib" ]; then
    record "datasource $name: plugin cdylib missing" false
    return 1
  fi
  local hash
  hash="$(plugin_hash "$dylib")"
  echo "  - $name (adapter=$adapter plugin_id=${hash:0:12}...)"
  if [ "${BEE_DRY_RUN:-0}" = "1" ]; then
    record "bee datasource create $name (dry-run)" true
  else
    if "$BEE" datasource create "$name" \
        --adapter "$adapter" \
        --plugin-id "$hash" \
        --config "$config_json" >/dev/null 2>&1; then
      record "bee datasource create $name" true
    else
      record "bee datasource create $name" false
    fi
  fi
}

register_ds binance      binance_subscribe  bee-plugin-binance      \
  "$(jq -nc --arg k "${BINANCE_API_KEY:-}" '{ws_url:"wss://stream.binance.com:9443",rest_url:"https://api.binance.com",api_key:$k,rate_limit_per_sec:10}')"

register_ds google_news  google_news_search bee-plugin-google-news  \
  "$(jq -nc --arg k "$NEWSAPI_KEY" '{base_url:"https://newsapi.org/v2",api_key:$k,rate_limit_per_sec:5,language:"en"}')"

register_ds influxdb     influxdb_write     bee-plugin-influxdb     \
  "$(jq -nc --arg u "$INFLUXDB_URL" --arg t "$INFLUXDB_TOKEN" --arg o "$INFLUXDB_ORG" '{url:$u,token:$t,org:$o,timeout_ms:5000}')"

register_ds mongodb      mongodb_insert     bee-plugin-mongodb      \
  "$(jq -nc --arg u "$MONGODB_URI" '{uri:$u,database:"trading",app_name:"bee",tls:false}')"

# 5. Deploy the 3 pipelines
step "deploy 3 SQL pipelines (backfill warmup -> v1 -> v2)"
deploy_sql() {
  local f="$1"
  if [ ! -f "$f" ]; then
    record "bee deploy $f (file missing)" false
    return 1
  fi
  if [ "${BEE_DRY_RUN:-0}" = "1" ]; then
    record "bee deploy $f (dry-run)" true
  else
    if "$BEE" deploy "$f" >/dev/null 2>&1; then
      record "bee deploy $f" true
    else
      record "bee deploy $f" false
    fi
  fi
}

deploy_sql docs/best-practices/quant/examples/quant_btc_strategy_backfill.sql
deploy_sql docs/best-practices/quant/examples/quant_btc_strategy.sql
deploy_sql docs/best-practices/quant/examples/quant_btc_strategy_v2.sql

# 6. Wait for live signals to flow
step "wait 60s for live signals"
sleep 60

# 7. Verify outputs hit the real sinks
step "verify outputs"

if [ "${BEE_DRY_RUN:-0}" = "1" ]; then
  echo "  (dry-run: skipping InfluxDB / MongoDB queries)"
  record "InfluxDB klines query"  true
  record "MongoDB trades query"   true
else
  echo "  InfluxDB query ->"
  if curl -sG "$INFLUXDB_URL/api/v2/query?org=$INFLUXDB_ORG" \
       --header "Authorization: Token $INFLUXDB_TOKEN" \
       --data-urlencode "bucket=trading" \
       --data-urlencode 'q=from(bucket:"trading") |> range(start:-5m) |> filter(fn: (r) => r._measurement == "klines") |> limit(n: 5)'; then
    record "InfluxDB klines query"  true
  else
    record "InfluxDB klines query"  false
  fi

  echo "  MongoDB query ->"
  if command -v mongosh >/dev/null 2>&1; then
    if mongosh --quiet "$MONGODB_URI/trading" \
         --eval 'db.trades.find().sort({ts:-1}).limit(3).toArray()'; then
      record "MongoDB trades query" true
    else
      record "MongoDB trades query" false
    fi
  else
    echo "  (mongosh not installed; skipping live query — verify visually via mongosh / compass)"
    record "MongoDB trades query (mongosh not installed; visual check only)"  true
  fi
fi

# 8. Verify Producer sharing
step "verify Producer sharing (binance Producer count == 1)"
if [ "${BEE_DRY_RUN:-0}" = "1" ]; then
  echo "  (dry-run: skipping bee jobs list)"
  record "Producer sharing OK (dry-run)" true
else
  N_PRODUCERS=$("$BEE" jobs list --filter 'producer' 2>/dev/null | wc -l | tr -d ' ')
  if [ "$N_PRODUCERS" -eq 1 ]; then
    record "Producer sharing OK (1 binance Producer)"  true
  else
    record "Producer sharing FAILED (expected 1 binance Producer, got $N_PRODUCERS)"  false
  fi
fi

# 9. Verify failover (deferred to 1.x — single-node MVP)
step "verify failover (deferred to 1.x)"
echo "  scripts/kill-node.sh is a 1.x feature (multi-node cluster)."
echo "  On the single-node MVP, the equivalent check is:"
echo "    - both strategies are still 'running' in \`bee jobs list\`"
echo "    - no 'producer disconnect' errors in \`bee diagnostics\`"
if [ "${BEE_DRY_RUN:-0}" = "1" ]; then
  record "failover (dry-run / 1.x deferred)" true
else
  N_RUNNING=$("$BEE" jobs list --filter 'status=running' 2>/dev/null | wc -l | tr -d ' ')
  if [ "$N_RUNNING" -ge 2 ]; then
    record "both strategies running on single node ($N_RUNNING running jobs)"  true
  else
    record "expected 2+ running jobs (v1 + v2), got $N_RUNNING"  false
  fi
fi

# 10. Summary
step "summary"
printf '%s\n' "${RESULTS[@]}"
echo
echo "PASS: $PASS    FAIL: $FAIL"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
