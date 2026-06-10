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

# Don't use `set -e` here: we want to keep going past a failing
# demo so the perf table at the end reports the status of all
# three demos (per the S41 acceptance criteria — measured numbers
# for the demos that worked). We still want `pipefail` so a
# pipeline that fails on the inner command is treated as a
# failure.
set -o pipefail
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
FIB_OUTPUT=$($BEE run examples/performance/fibonacci.sql 2>&1) || true
FIB_OK=$([ -n "$FIB_OUTPUT" ] && [ "${FIB_OUTPUT##*emitted*}" != "$FIB_OUTPUT" ] && echo 1 || echo 0)
T1=$(date +%s%N)
FIB_MS=$(( (T1 - T0) / 1000000 ))
FIB_TPUT=$(( 1000000 * 1000000000 / (T1 - T0) ))
echo "$FIB_OUTPUT" | tail -10
if [ "$FIB_OK" -ne 1 ]; then
    echo "FAIL: fibonacci demo did not run to completion"
else
    echo "✓ fibonacci demo complete (first 20 values printed above)"
fi

# 2. Demo 2: prime sieve
echo ""
echo "==== Demo 2: Prime sieve (≤ 10^8, 1229 sieving phases — full Eratosthenes) ===="
T0=$(date +%s%N)
PRIME_OUTPUT=$($BEE run examples/performance/prime_sieve.sql 2>&1) || true
T1=$(date +%s%N)
SIEVE_MS=$(( (T1 - T0) / 1000000 ))
echo "$PRIME_OUTPUT" | tail -5

# Hard correctness check: with 1229 phases, the count is the true prime count
# of 5,761,455 (primes ≤ 10^8). See SQL header comment for the math.
EXPECTED_COUNT=5761455
N=$(echo "$PRIME_OUTPUT" | grep -oE 'n_primes=[0-9]+' | tail -1 | cut -d= -f2)
SIEVE_OK=1
if [ -z "$N" ]; then
    echo "FAIL: count not found in output (sieve did not run to completion)"
    SIEVE_OK=0
elif [ "$N" -ne "$EXPECTED_COUNT" ]; then
    echo "FAIL: count mismatch (expected $EXPECTED_COUNT, got $N)"
    SIEVE_OK=0
else
    echo "✓ count correct ($EXPECTED_COUNT)"
fi

# 3. Demo 3: multi-stream analytics
echo ""
echo "==== Demo 3: Multi-stream analytics (160K events) ===="
T0=$(date +%s%N)
MS_OUTPUT=$($BEE run examples/performance/multi_stream_analytics.sql 2>&1) || true
MS_OK=$([ -n "$MS_OUTPUT" ] && [ "${MS_OUTPUT##*emitted*}" != "$MS_OUTPUT" ] && echo 1 || echo 0)
T1=$(date +%s%N)
MS_MS=$(( (T1 - T0) / 1000000 ))
MS_TPUT=$(( 160000 * 1000000000 / (T1 - T0) ))
echo "$MS_OUTPUT" | tail -10
if [ "$MS_OK" -ne 1 ]; then
    echo "FAIL: multi-stream analytics demo did not run to completion"
else
    echo "✓ multi-stream analytics demo complete"
fi

# 4. Print measured perf table
cat <<EOF

==== Measured performance (1 Node) ====
| Demo                      | Wall-clock        | Throughput              | Status    |
|---------------------------|-------------------|-------------------------|-----------|
| Fibonacci (1M values)     | ${FIB_MS} ms      | ${FIB_TPUT} events/sec  | $([ "$FIB_OK" = 1 ] && echo "ok" || echo "FAIL")    |
| Prime sieve (≤ 10^8)      | ${SIEVE_MS} ms    | (10^8 ints sieved)      | $([ "$SIEVE_OK" = 1 ] && echo "ok" || echo "FAIL")    |
| Multi-stream analytics    | ${MS_MS} ms       | ${MS_TPUT} events/sec   | $([ "$MS_OK" = 1 ] && echo "ok" || echo "FAIL")    |

EOF

# Exit non-zero if any demo failed. The script can be inspected to see
# which demo's status is "FAIL" in the table above.
if [ "$FIB_OK" -ne 1 ] || [ "$SIEVE_OK" -ne 1 ] || [ "$MS_OK" -ne 1 ]; then
    exit 1
fi
