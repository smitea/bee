use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeAddr {
    pub id: String,
    pub addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollingRestartPlan {
    pub nodes: Vec<NodeAddr>,
    pub batch_size: u32,
    pub health_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepResult {
    pub restarted: Vec<String>,
    pub failed: Option<String>,
    pub done: bool,
    pub next_step: u32,
}

pub const DEFAULT_BATCH_SIZE: u32 = 1;
pub const DEFAULT_HEALTH_TIMEOUT_MS: u64 = 30_000;

pub fn plan(nodes: Vec<NodeAddr>) -> RollingRestartPlan {
    RollingRestartPlan {
        nodes,
        batch_size: DEFAULT_BATCH_SIZE,
        health_timeout_ms: DEFAULT_HEALTH_TIMEOUT_MS,
    }
}

pub fn plan_with_leader(nodes: Vec<NodeAddr>, leader_id: Option<&str>) -> RollingRestartPlan {
    let mut sorted = nodes;
    if let Some(leader) = leader_id {
        sorted.sort_by(|a, b| {
            if a.id == leader {
                std::cmp::Ordering::Greater
            } else if b.id == leader {
                std::cmp::Ordering::Less
            } else {
                a.id.cmp(&b.id)
            }
        });
    }
    RollingRestartPlan {
        nodes: sorted,
        batch_size: DEFAULT_BATCH_SIZE,
        health_timeout_ms: DEFAULT_HEALTH_TIMEOUT_MS,
    }
}

pub fn simulate_step(plan: &RollingRestartPlan, step: u32) -> StepResult {
    if plan.nodes.is_empty() {
        return StepResult {
            restarted: vec![],
            failed: None,
            done: true,
            next_step: 0,
        };
    }
    let idx = step as usize;
    if idx >= plan.nodes.len() {
        return StepResult {
            restarted: vec![],
            failed: None,
            done: true,
            next_step: step,
        };
    }
    let restart_count = (plan.batch_size as usize).max(1).min(plan.nodes.len() - idx);
    let restarted: Vec<String> = plan.nodes[idx..idx + restart_count]
        .iter()
        .map(|n| n.id.clone())
        .collect();
    let next_idx = idx + restart_count;
    let done = next_idx >= plan.nodes.len();
    StepResult {
        restarted,
        failed: None,
        done,
        next_step: next_idx as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: &str, addr: &str) -> NodeAddr {
        NodeAddr {
            id: id.into(),
            addr: addr.into(),
        }
    }

    #[test]
    fn plan_with_empty_nodes_has_default_batch_and_timeout() {
        let p = plan(vec![]);
        assert!(p.nodes.is_empty());
        assert_eq!(p.batch_size, 1);
        assert_eq!(p.health_timeout_ms, 30_000);
    }

    #[test]
    fn plan_preserves_input_order_when_no_leader() {
        let nodes = vec![n("a", "1.1.1.1:1"), n("b", "1.1.1.1:2")];
        let p = plan(nodes.clone());
        assert_eq!(p.nodes, nodes);
    }

    #[test]
    fn plan_with_leader_sorts_leader_last_preserving_quorum() {
        let nodes = vec![
            n("a", "1.1.1.1:1"),
            n("b-leader", "1.1.1.1:2"),
            n("c", "1.1.1.1:3"),
            n("d", "1.1.1.1:4"),
        ];
        let p = plan_with_leader(nodes, Some("b-leader"));
        let ids: Vec<&str> = p.nodes.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c", "d", "b-leader"]);
    }

    #[test]
    fn plan_with_leader_id_not_in_list_preserves_stable_order() {
        let nodes = vec![n("a", "1"), n("b", "2"), n("c", "3")];
        let p = plan_with_leader(nodes.clone(), Some("nonexistent"));
        assert_eq!(p.nodes, nodes);
    }

    #[test]
    fn simulate_step_empty_plan_returns_done_immediately() {
        let p = plan(vec![]);
        let r = simulate_step(&p, 0);
        assert!(r.restarted.is_empty());
        assert!(r.failed.is_none());
        assert!(r.done);
        assert_eq!(r.next_step, 0);
    }

    #[test]
    fn simulate_step_first_step_restarts_first_node() {
        let p = plan(vec![n("a", "1"), n("b", "2"), n("c", "3")]);
        let r = simulate_step(&p, 0);
        assert_eq!(r.restarted, vec!["a".to_string()]);
        assert!(r.failed.is_none());
        assert!(!r.done);
        assert_eq!(r.next_step, 1);
    }

    #[test]
    fn simulate_step_middle_step_restarts_middle_node() {
        let p = plan(vec![n("a", "1"), n("b", "2"), n("c", "3")]);
        let r = simulate_step(&p, 1);
        assert_eq!(r.restarted, vec!["b".to_string()]);
        assert!(!r.done);
        assert_eq!(r.next_step, 2);
    }

    #[test]
    fn simulate_step_last_step_returns_done() {
        let p = plan(vec![n("a", "1"), n("b", "2")]);
        let r = simulate_step(&p, 1);
        assert_eq!(r.restarted, vec!["b".to_string()]);
        assert!(r.done);
        assert_eq!(r.next_step, 2);
    }

    #[test]
    fn simulate_step_past_end_returns_done_no_restart() {
        let p = plan(vec![n("a", "1"), n("b", "2")]);
        let r = simulate_step(&p, 5);
        assert!(r.restarted.is_empty());
        assert!(r.done);
        assert_eq!(r.next_step, 5);
    }

    #[test]
    fn simulate_step_through_all_nodes_in_order() {
        let p = plan(vec![
            n("a", "1"),
            n("b", "2"),
            n("c", "3"),
            n("d-leader", "4"),
        ]);
        let r0 = simulate_step(&p, 0);
        assert_eq!(r0.restarted, vec!["a".to_string()]);
        let r1 = simulate_step(&p, 1);
        assert_eq!(r1.restarted, vec!["b".to_string()]);
        let r2 = simulate_step(&p, 2);
        assert_eq!(r2.restarted, vec!["c".to_string()]);
        let r3 = simulate_step(&p, 3);
        assert_eq!(r3.restarted, vec!["d-leader".to_string()]);
        assert!(r3.done);
    }

    #[test]
    fn simulate_step_respects_batch_size_greater_than_one() {
        let mut p = plan(vec![
            n("a", "1"),
            n("b", "2"),
            n("c", "3"),
            n("d", "4"),
        ]);
        p.batch_size = 2;
        let r0 = simulate_step(&p, 0);
        assert_eq!(r0.restarted, vec!["a".to_string(), "b".to_string()]);
        assert!(!r0.done);
        assert_eq!(r0.next_step, 2);
        let r1 = simulate_step(&p, 2);
        assert_eq!(r1.restarted, vec!["c".to_string(), "d".to_string()]);
        assert!(r1.done);
    }
}
