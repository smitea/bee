#!/usr/bin/env bash
# scripts/deploy-plugins.sh — Deploy workspace plugins across Bee cluster nodes
#
# Usage:
#   scripts/deploy-plugins.sh [--build] [--profile release|debug] [--mode docker|volume|local]
#
# Modes:
#   docker   Copies plugins into docker volume dirs (./volumes/node_N/plugins) and `docker cp` into running containers
#   volume   Copies plugins into docker volume dirs (./volumes/node_N/plugins)
#   local    Copies plugins into local process dirs (/tmp/bee_plugins/node_N)
#
# Defaults: --profile release, --mode docker, partitioned distribution.
#
# Plugins distributed (must exist as workspace members):
#   libbee_plugin_onnx_ml        - plugins/bee-plugin-onnx-ml        (heavy: tract-onnx + FinBERT)
#   libbee_plugin_perf_fib       - plugins/bee-plugin-perf-fib       (lightweight perf handler)
#   libbee_plugin_sample_kline   - plugins/sample-kline              (lightweight K-line demo)

set -euo pipefail
cd "$(dirname "$0")/.."

BUILD_PLUGINS=false
PROFILE="release"
MODE="docker"

while [ $# -gt 0 ]; do
    case "$1" in
        --build) BUILD_PLUGINS=true; shift ;;
        --profile) PROFILE="$2"; shift 2 ;;
        --mode) MODE="$2"; shift 2 ;;
        *) echo "Unknown flag: $1" >&2; exit 2 ;;
    esac
done

TARGET_DIR="target/$PROFILE"

OS_NAME=$(uname -s)
case "$OS_NAME" in
    Linux*)     LIB_EXT="so";;
    Darwin*)    LIB_EXT="dylib";;
    CYGWIN*|MINGW*|MSYS*) LIB_EXT="dll";;
    *)          LIB_EXT="so";;
esac

PLUGINS=(
    "libbee_plugin_onnx_ml"
    "libbee_plugin_perf_fib"
    "libbee_plugin_sample_kline"
)

if [ "$BUILD_PLUGINS" = true ]; then
    echo "Building plugins ($PROFILE profile)..."
    CARGO_FLAGS=""
    if [ "$PROFILE" = "release" ]; then
        CARGO_FLAGS="--release"
    fi
    cargo build $CARGO_FLAGS \
        -p bee-plugin-onnx-ml \
        -p bee-plugin-perf-fib \
        -p bee-plugin-sample-kline
fi

get_target_dir() {
    local node_id="$1"
    if [ "$MODE" = "local" ]; then
        echo "/tmp/bee_plugins/node_${node_id}"
    else
        echo "./volumes/node_${node_id}/plugins"
    fi
}

# Partitioned distribution. Node 5 keeps all three as a warm pool so it can
# absorb work-stealing / failover from any other node.
get_plugins_for_node() {
    local node_id="$1"
    case "$node_id" in
        1) echo "libbee_plugin_sample_kline" ;;
        2) echo "libbee_plugin_perf_fib" ;;
        3) echo "libbee_plugin_onnx_ml" ;;
        4) echo "" ;;
        5) echo "${PLUGINS[*]}" ;;
    esac
}

for i in $(seq 1 5); do
    mkdir -p "$(get_target_dir "$i")"
done

echo "Deploying plugins to Bee cluster nodes (mode: $MODE)..."
echo "--------------------------------------------------------"

for i in $(seq 1 5); do
    DEST_DIR="$(get_target_dir "$i")"
    mkdir -p "$DEST_DIR"

    echo "Node $i ($DEST_DIR):"
    NODE_PLUGIN_LIST=$(get_plugins_for_node "$i")
    if [ -z "$NODE_PLUGIN_LIST" ]; then
        echo "   (none)"
        continue
    fi
    for plugin in $NODE_PLUGIN_LIST; do
        SRC_FILE=""
        if [ -f "$TARGET_DIR/${plugin}.${LIB_EXT}" ]; then
            SRC_FILE="$TARGET_DIR/${plugin}.${LIB_EXT}"
        elif [ -f "$TARGET_DIR/${plugin}.so" ]; then
            SRC_FILE="$TARGET_DIR/${plugin}.so"
        elif [ -f "$TARGET_DIR/${plugin}.dylib" ]; then
            SRC_FILE="$TARGET_DIR/${plugin}.dylib"
        fi

        if [ -n "$SRC_FILE" ]; then
            cp "$SRC_FILE" "$DEST_DIR/"
            FILENAME=$(basename "$SRC_FILE")
            echo "   + Deployed $FILENAME"

            if [ "$MODE" = "docker" ] && command -v docker >/dev/null 2>&1; then
                CONTAINER_NAME="bee-node-${i}"
                if docker ps --format '{{.Names}}' 2>/dev/null | grep -q "^${CONTAINER_NAME}$"; then
                    docker cp "$SRC_FILE" "${CONTAINER_NAME}:/etc/bee/plugins/" 2>/dev/null || true
                    echo "     (Synced to active container ${CONTAINER_NAME})"
                fi
            fi
        else
            echo "   Warning: $plugin not found in $TARGET_DIR. Run with --build flag to compile."
        fi
    done
done

echo "--------------------------------------------------------"
echo "Plugin deployment complete."