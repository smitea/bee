//! Unit tests for the `fib_seed` UDF (stateless).

use bee_plugin_perf_fib::fib_seed;

#[test]
fn fib_seed_returns_0_for_n_0() {
    assert_eq!(fib_seed(0), 0);
}

#[test]
fn fib_seed_returns_1_for_n_1() {
    assert_eq!(fib_seed(1), 1);
}

#[test]
fn fib_seed_returns_1_for_n_ge_1() {
    // Per the UDF spec: fib_seed returns 0 only for n=0; for n>=1, returns 1.
    for n in 1..20 {
        assert_eq!(fib_seed(n), 1, "fib_seed({}) should be 1", n);
    }
}
