use bee_control::{KVStateMachine, Op, TxnError};

#[test]
fn put_then_get_returns_value() {
    let mut sm = KVStateMachine::new();
    sm.put("alpha".to_string(), b"one".to_vec());
    sm.put("beta".to_string(), b"two".to_vec());
    assert_eq!(sm.get("alpha"), Some(b"one".to_vec()));
    assert_eq!(sm.get("beta"), Some(b"two".to_vec()));
    assert_eq!(sm.get("missing"), None);
}

#[test]
fn del_removes_value() {
    let mut sm = KVStateMachine::new();
    sm.put("k".to_string(), b"v".to_vec());
    assert_eq!(sm.del("k"), Some(b"v".to_vec()));
    assert_eq!(sm.get("k"), None);
    assert_eq!(sm.del("k"), None);
}

#[test]
fn cas_succeeds_when_expected_matches() {
    let mut sm = KVStateMachine::new();
    sm.put("k".to_string(), b"v1".to_vec());
    assert!(sm.cas("k", Some(b"v1"), b"v2".to_vec()));
    assert_eq!(sm.get("k"), Some(b"v2".to_vec()));
}

#[test]
fn cas_rejects_mismatched_expected() {
    let mut sm = KVStateMachine::new();
    sm.put("k".to_string(), b"v1".to_vec());
    assert!(!sm.cas("k", Some(b"v0"), b"v2".to_vec()));
    assert_eq!(sm.get("k"), Some(b"v1".to_vec()), "value must be unchanged on mismatch");
}

#[test]
fn cas_expected_none_only_succeeds_when_key_absent() {
    let mut sm = KVStateMachine::new();
    assert!(sm.cas("new", None, b"v".to_vec()));
    assert_eq!(sm.get("new"), Some(b"v".to_vec()));
    assert!(!sm.cas("new", None, b"v2".to_vec()));
    assert_eq!(sm.get("new"), Some(b"v".to_vec()));
}

#[test]
fn cas_checked_returns_conflict_with_actual_value() {
    let mut sm = KVStateMachine::new();
    sm.put("k".to_string(), b"v1".to_vec());
    let err = sm
        .cas_checked("k", Some(b"WRONG"), b"v2".to_vec())
        .expect_err("must fail");
    match err {
        TxnError::Conflict { key, expected, actual } => {
            assert_eq!(key, "k");
            assert_eq!(expected, Some(b"WRONG".to_vec()));
            assert_eq!(actual, Some(b"v1".to_vec()));
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn txn_applies_all_ops_when_all_succeed() {
    let mut sm = KVStateMachine::new();
    sm.put("a".to_string(), b"1".to_vec());

    let ops = vec![
        Op::Put { key: "b".to_string(), value: b"2".to_vec() },
        Op::Cas { key: "a".to_string(), expected: Some(b"1".to_vec()), new: b"11".to_vec() },
        Op::Put { key: "c".to_string(), value: b"3".to_vec() },
    ];
    sm.txn(ops).expect("txn must succeed");
    assert_eq!(sm.get("a"), Some(b"11".to_vec()));
    assert_eq!(sm.get("b"), Some(b"2".to_vec()));
    assert_eq!(sm.get("c"), Some(b"3".to_vec()));
}

#[test]
fn txn_rejects_all_ops_when_any_cas_mismatches() {
    let mut sm = KVStateMachine::new();
    sm.put("a".to_string(), b"1".to_vec());

    let ops = vec![
        Op::Put { key: "b".to_string(), value: b"2".to_vec() },
        Op::Cas { key: "a".to_string(), expected: Some(b"WRONG".to_vec()), new: b"X".to_vec() },
        Op::Put { key: "c".to_string(), value: b"3".to_vec() },
    ];
    let err = sm.txn(ops).expect_err("txn must fail");
    assert!(matches!(err, TxnError::Conflict { .. }));
    assert_eq!(sm.get("a"), Some(b"1".to_vec()), "a must be unchanged");
    assert_eq!(sm.get("b"), None, "b must not be applied");
    assert_eq!(sm.get("c"), None, "c must not be applied");
}

#[test]
fn txn_rejects_nested_transactions() {
    let mut sm = KVStateMachine::new();
    let ops = vec![
        Op::Put { key: "a".to_string(), value: b"1".to_vec() },
        Op::Txn { ops: vec![Op::Put { key: "b".to_string(), value: b"2".to_vec() }] },
    ];
    let err = sm.txn(ops).expect_err("nested txn must be rejected");
    assert!(matches!(err, TxnError::NestedTxn));
}
