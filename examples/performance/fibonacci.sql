-- Fibonacci (1M values): exercises stateful Handler UDF + KV-backed state.
-- This is the S41 demo's smallest possible streaming-compute surface.

use perf_fib;

CREATE SOURCE naturals AS
SELECT n FROM generate_series(1, 1000000);

CREATE VIEW fib_stream AS
SELECT
    n,
    fib_step(n) AS fib_value
FROM naturals;

-- Sanity check: emit the first 20 fib values to the console.
-- Note: with initial state FibState { prev2: 0, prev1: 1 } (per the
-- FibState docstring), fib_step(1) = 0+1 = 1, fib_step(2) = 1+1 = 2,
-- fib_step(3) = 2+1 = 3, etc. So the first 20 emitted values are:
-- 1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765, 10946
EMIT INTO console
SELECT n, fib_value FROM fib_stream WHERE n <= 20;
