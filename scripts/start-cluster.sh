#!/usr/bin/env bash
# scripts/start-cluster.sh — S33.1: start N `bee node` processes
# for a multi-node Bee cluster. Records PIDs in
# /tmp/bee_cluster.pids; prints the leader once elected.
#
# Usage:
#   scripts/start-cluster.sh [--nodes N] [--bind 127.0.0.1] [--base-port 7701]
#
# Defaults: --nodes 3, --bind 127.0.0.1, --base-port 7701.
# Each node N listens on `$bind:$((base_port + N - 1))`.
# Peer addresses are derived from --bind + --base-port.
#
# Note (S33.1 MVP): the per-Node AdminServer (Task 7) is
# not yet wired into `bee node` (run_node.rs), so the
# "wait for leader" step falls back to log-based
# detection: each node prints "bee node N listening on
# $bind:$port (peers: ...)" on startup, and Raft's
# election timeout (default 800ms) elects a leader
# within ~3s. S33.2 will wire AdminServer into run_node
# and switch the wait step to a `--connect cluster
# status` probe.
#
# Exit codes:
#   0   leader elected within 10s
#   1   no leader elected within 10s (check /tmp/bee_logs/)
#   2   bad flags

set -euo pipefail
cd "$(dirname "$0")/.."

NODES=3
BIND=127.0.0.1
BASE_PORT=7701
while [ $# -gt 0 ]; do
    case "$1" in
        --nodes) NODES="$2"; shift 2 ;;
        --bind) BIND="$2"; shift 2 ;;
        --base-port) BASE_PORT="$2"; shift 2 ;;
        *) echo "unknown flag $1" >&2; exit 2 ;;
    esac
done

# Use the dev profile by default; the release build
# is much slower to build and the demo only needs
# the dev profile. Override with BEE_PROFILE=release.
PROFILE_FLAG=""
if [ "${BEE_PROFILE:-dev}" = "release" ]; then
    PROFILE_FLAG="--release"
fi
BEE=./target/release/bee
if [ "${BEE_PROFILE:-dev}" != "release" ]; then
    BEE=./target/debug/bee
fi
if [ ! -x "$BEE" ]; then
    echo "building bee (${BEE_PROFILE:-dev})..." >&2
    cargo build $PROFILE_FLAG --quiet -p bee
fi

mkdir -p /tmp/bee_logs
rm -f /tmp/bee_cluster.pids
: > /tmp/bee_cluster.pids

# Spawn N nodes.
for i in $(seq 1 "$NODES"); do
    RAFT_PORT=$((BASE_PORT + i - 1))
    # Build the --peer flags.
    PEER_FLAGS=""
    for j in $(seq 1 "$NODES"); do
        if [ "$j" != "$i" ]; then
            PEER_PORT=$((BASE_PORT + j - 1))
            PEER_FLAGS="$PEER_FLAGS --peer $j=$BIND:$PEER_PORT"
        fi
    done
    LOG=/tmp/bee_logs/node_$i.log
    echo "starting node $i (raft $BIND:$RAFT_PORT) → $LOG" >&2
    "$BEE" node --id "$i" --bind "$BIND:$RAFT_PORT" $PEER_FLAGS \
        > "$LOG" 2>&1 &
    echo "$i $!" >> /tmp/bee_cluster.pids
done

# Wait for leader election. With the default
# base_election_timeout=800ms and a 3-node cluster, a
# leader is typically elected within 1-3s; we allow
# 10s for CI variance.
echo "waiting for leader election..." >&2
DEADLINE=$((SECONDS + 10))
LEADER=""
while [ $SECONDS -lt $DEADLINE ]; do
    # Each node's log prints "bee node N listening on
    # ..." on bind; the leader is whichever one wins
    # the election. Without AdminServer wiring, we
    # can't query cluster state directly, so we
    # declare success when:
    #   1. all 3 nodes have logged "listening on"
    #   2. at least 3s has passed (so an election has
    #      had a chance to run with the default 800ms
    #      timeout × ~2-3 election rounds)
    N_LISTENING=0
    for i in $(seq 1 "$NODES"); do
        if grep -q "listening on" "/tmp/bee_logs/node_$i.log" 2>/dev/null; then
            N_LISTENING=$((N_LISTENING + 1))
        fi
    done
    if [ "$N_LISTENING" -eq "$NODES" ] && [ $((SECONDS + 10 - DEADLINE + SECONDS)) -ge 3 ]; then
        # All 3 nodes are up; pick node 1 as the
        # leader placeholder. S33.2 will replace this
        # with a real --connect cluster status probe
        # that returns the actual leader_id.
        LEADER=1
        break
    fi
    sleep 1
done

if [ -z "$LEADER" ]; then
    echo "ERROR: no leader elected within 10s; check /tmp/bee_logs/" >&2
    exit 1
fi
echo "leader: node $LEADER (placeholder; S33.2 wires AdminServer for real detection)"
echo "pids: $(awk '{printf "%s=%s ", $1, $2}' /tmp/bee_cluster.pids)"
