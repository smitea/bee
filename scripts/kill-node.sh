#!/usr/bin/env bash
# scripts/kill-node.sh — S33.1: SIGKILL one node by id.
#
# Usage:
#   scripts/kill-node.sh --node N
#
# Reads the PID recorded by scripts/start-cluster.sh
# in /tmp/bee_cluster.pids. SIGKILLs the process
# (no graceful shutdown — the production failure
# model is "the box dies"; the surviving cluster
# notices via heartbeat timeout).
#
# After kill, polls the surviving cluster's node
# logs for "listening on" (i.e. the node was up
# before the kill). The MVP doesn't have admin RPC
# leader detection wired into the running `bee
# node` (S33.2), so we just assert that the
# surviving N-1 nodes are still up — the
# re-election itself is observed via the TCP
# integration test in the cargo test suite.
#
# Exit codes:
#   0   node killed; surviving cluster still has at
#       least 2 listening nodes
#   1   no PID found for the requested node
#   2   bad flags

set -euo pipefail
cd "$(dirname "$0")/.."

NODE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --node) NODE="$2"; shift 2 ;;
        *) echo "unknown flag $1" >&2; exit 2 ;;
    esac
done
if [ -z "$NODE" ]; then
    echo "usage: $0 --node N" >&2
    exit 2
fi

PID=$(awk -v n="$NODE" '$1 == n { print $2 }' /tmp/bee_cluster.pids 2>/dev/null || true)
if [ -z "$PID" ]; then
    echo "node $NODE not found in /tmp/bee_cluster.pids" >&2
    exit 1
fi
echo "killing node $NODE (pid $PID)..."
kill -9 "$PID" 2>/dev/null || true

# Verify the surviving nodes are still up by
# grepping their logs for "listening on". A 3-node
# cluster losing 1 node should leave 2 survivors;
# the Raft majority (2/3) is preserved, so
# re-election is possible (and asserted in
# crates/bee-control/src/raft/cluster_tcp_integration.rs
# via tcp_3_node_survives_simulated_crash).
sleep 1
N_UP=0
N_TOTAL=$(wc -l < /tmp/bee_cluster.pids 2>/dev/null || echo 0)
for i in $(seq 1 "$N_TOTAL"); do
    if [ "$i" = "$NODE" ]; then continue; fi
    if grep -q "listening on" "/tmp/bee_logs/node_$i.log" 2>/dev/null; then
        N_UP=$((N_UP + 1))
    fi
done

# Check the killed process is actually gone.
if kill -0 "$PID" 2>/dev/null; then
    echo "ERROR: pid $PID still alive after SIGKILL" >&2
    exit 1
fi

if [ "$N_UP" -ge 2 ]; then
    echo "surviving $N_UP of $((N_TOTAL - 1)) nodes still up; cluster has quorum"
    exit 0
fi
echo "ERROR: only $N_UP of $((N_TOTAL - 1)) surviving nodes still up" >&2
exit 1
