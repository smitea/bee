//! S33.5.3 Task 2: locks down the
//! `extract_phase_dag` function.

use bee_dsl_sql::dag::extract_phase_dag;

#[test]
fn extracts_two_phases_from_two_selects() {
    let sql = "SELECT * FROM binance.subscribe('BTC/USDT', '5min'); \
               SELECT avg(price) FROM ticks;";
    let dag = extract_phase_dag(sql).expect("extract");
    assert_eq!(dag.phases.len(), 2);
    assert_eq!(dag.phases[0].phase_id, 1);
    assert_eq!(dag.phases[1].phase_id, 2);
    assert!(dag.dependencies.is_empty());
    assert_eq!(dag.dag_hash.len(), 64, "sha256 hex is 64 chars");
    // Same SQL → same hash (idempotency).
    let dag2 = extract_phase_dag(sql).expect("extract");
    assert_eq!(dag.dag_hash, dag2.dag_hash);
}

#[test]
fn errors_on_empty_sql() {
    let dag = extract_phase_dag("");
    assert!(dag.is_err());
    let err = dag.unwrap_err();
    assert!(
        err.contains("parse failed") || err.contains("no SELECT"),
        "expected parse or empty error, got: {err}"
    );
}

#[test]
fn errors_on_no_selects() {
    // SET is a non-SELECT statement.
    let sql = "SET foo = 1;";
    let dag = extract_phase_dag(sql);
    assert!(dag.is_err());
    let err = dag.unwrap_err();
    assert!(
        err.contains("no SELECT"),
        "expected 'no SELECT' error, got: {err}"
    );
}
