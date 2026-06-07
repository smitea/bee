use bee_control::{Command, Op, RaftNode, TxnError};
use tokio::sync::{mpsc, oneshot};

#[tokio::test]
async fn raft_loop_accepts_commands_via_channel() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(8);
    let node = RaftNode::new();
    let handle = tokio::spawn(node.run(cmd_rx));

    let (reply_tx, reply_rx) = oneshot::channel();
    cmd_tx
        .send(Command {
            op: Op::Put { key: "k".to_string(), value: b"v".to_vec() },
            reply: Some(reply_tx),
        })
        .await
        .unwrap();
    let result = reply_rx.await.unwrap();
    assert!(result.is_ok());

    drop(cmd_tx);
    handle.await.unwrap();
}

#[tokio::test]
async fn raft_loop_applies_put_cas_success_cas_failure_via_channel() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(8);
    let node = RaftNode::new();
    let handle = tokio::spawn(node.run(cmd_rx));

    let (reply_tx, reply_rx) = oneshot::channel();
    cmd_tx
        .send(Command {
            op: Op::Put { key: "k".to_string(), value: b"v".to_vec() },
            reply: Some(reply_tx),
        })
        .await
        .unwrap();
    assert!(reply_rx.await.unwrap().is_ok());

    let (reply_tx, reply_rx) = oneshot::channel();
    cmd_tx
        .send(Command {
            op: Op::Cas {
                key: "k".to_string(),
                expected: Some(b"v".to_vec()),
                new: b"v2".to_vec(),
            },
            reply: Some(reply_tx),
        })
        .await
        .unwrap();
    assert!(reply_rx.await.unwrap().is_ok(), "matching cas must succeed");

    let (reply_tx, reply_rx) = oneshot::channel();
    cmd_tx
        .send(Command {
            op: Op::Cas {
                key: "k".to_string(),
                expected: Some(b"WRONG".to_vec()),
                new: b"v3".to_vec(),
            },
            reply: Some(reply_tx),
        })
        .await
        .unwrap();
    let err = reply_rx.await.unwrap().expect_err("mismatched cas must fail");
    assert!(matches!(err, TxnError::Conflict { .. }));

    drop(cmd_tx);
    handle.await.unwrap();
}

#[test]
fn raft_node_synchronous_apply_put_then_get_via_state_machine() {
    let mut node = RaftNode::new();
    assert_eq!(node.committed_index(), 0);

    node.apply(Op::Put { key: "k".to_string(), value: b"v".to_vec() })
        .unwrap();
    assert_eq!(node.committed_index(), 1);
    assert_eq!(
        node.state_machine().get("k"),
        Some(b"v".to_vec()),
        "put then get via RaftNode must return the value"
    );
}

#[test]
fn raft_node_apply_atomic_txn() {
    let mut node = RaftNode::new();
    node.apply(Op::Put { key: "a".to_string(), value: b"1".to_vec() })
        .unwrap();
    let initial_index = node.committed_index();

    let ops = vec![
        Op::Put { key: "b".to_string(), value: b"2".to_vec() },
        Op::Cas {
            key: "a".to_string(),
            expected: Some(b"WRONG".to_vec()),
            new: b"X".to_vec(),
        },
    ];
    assert!(node.apply(Op::Txn { ops }).is_err());
    assert_eq!(node.committed_index(), initial_index, "failed txn must not advance index");
    assert_eq!(node.state_machine().get("b"), None, "no partial apply");
}
