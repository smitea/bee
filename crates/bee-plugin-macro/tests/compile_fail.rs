//! S33.6 Task 6: trybuild compile-fail for
//! `#[bee_adapter(input)]` signature checks.

#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
