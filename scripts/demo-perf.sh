#!/usr/bin/env bash
# scripts/demo-perf.sh — S41 1-Node performance showcase.
#
# Runs the 3 S41 demos and prints a measured performance table.
# Pre-builds the perf-fib plugin and the bee binary first (to avoid
# cargo run startup overhead in the timing measurement).
#
# Usage:
#   scripts/demo-perf.sh            # run all 3 demos
#   BEE_QUIET=1 scripts/demo-perf.sh # less output

set -euo pipefail
cd "$(dirname "$0")/.."

# 0. Pre-build
echo "==== Pre-build ===="
(cd plugins/bee-plugin-perf-fib && cargo build --release --quiet)
cargo build --release --quiet -p bee-dsl-sql --features test-fixtures
cargo build --release --quiet -p bee

BEE=./target/release/bee

# Verify the binary exists
if [ ! -x "$BEE" ]; then
    echo "ERROR: $BEE not found after build"
    exit 1
fi

# 1. Demo 1: Fibonacci
echo ""
echo "==== Demo 1: Fibonacci (1M values) ===="
T0=$(date +%s%N)
$BEE run examples/performance/fibonacci.sql 2>&1 | tail -25
T1=$(date +%s%N)
FIB_MS=$(( (T1 - T0) / 1000000 ))
FIB_TPUT=$(( 1000000 * 1000000000 / (T1 - T0) ))

# 2. Demo 2: prime sieve
echo ""
echo "==== Demo 2: Prime sieve (≤ 10^8, 20 sieving phases) ===="
T0=$(date +%s%N)
PRIME_OUTPUT=$($BEE run examples/performance/prime_sieve.sql 2>&1)
T1=$(date +%s%N)
SIEVE_MS=$(( (T1 - T0) / 1000000 ))
echo "$PRIME_OUTPUT" | tail -5

# Hard correctness check: with 20 phases, the count is 12,779,448
# (not the true prime count of 5,761,455; see SQL header comment for math).
EXPECTED_COUNT=12779448
N=$(echo "$PRIME_OUTPUT" | grep -oE 'count=[0-9]+' | tail -1 | cut -d= -f2)
if [ -z "$N" ]; then
    echo "FAIL: count not found in output"
    exit 1
fi
if [ "$N" -ne "$EXPECTED_COUNT" ]; then
    echo "FAIL: count mismatch (expected $EXPECTED_COUNT, got $N)"
    exit 1
fi
echo "✓ count correct ($EXPECTED_COUNT)"

# 3. Demo 3: multi-stream analytics
echo ""
echo "==== Demo 3: Multi-stream analytics (160K events) ===="
T0=$(date +%s%N)
$BEE run examples/performance/multi_stream_analytics.sql 2>&1 | tail -25
T1=$(date +%s%N)
MS_MS=$(( (T1 - T0) / 1000000 ))
MS_TPUT=$(( 160000 * 1000000000 / (T1 - T0) ))

# 4. Print measured perf table
cat <<EOF

==== Measured performance (1 Node) ====
| Demo                      | Wall-clock        | Throughput              |
|---------------------------|-------------------|-------------------------|
| Fibonacci (1M values)     | ${FIB_MS} ms      | ${FIB_TPUT} events/sec  |
| Prime sieve (≤ 10^8)      | ${SIEVE_MS} ms    | (10^8 ints sieved)      |
| Multi-stream analytics    | ${MS_MS} ms       | ${MS_TPUT} events/sec   |

EOF
