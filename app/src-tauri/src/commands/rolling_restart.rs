use serde::Serialize;

use crate::commands::{CmdError, CmdResult};
use crate::rolling_restart::{plan, simulate_step, NodeAddr, RollingRestartPlan, StepResult};

#[derive(Debug, Serialize, Clone)]
pub struct ApplyReport {
    pub plan: RollingRestartPlan,
    pub steps: Vec<StepResult>,
    pub applied: bool,
}

#[tauri::command]
pub fn rolling_restart_apply(addr: String, nodes: Vec<NodeAddr>) -> CmdResult<RollingRestartPlan> {
    let _ = addr;
    if nodes.is_empty() {
        return Err(CmdError {
            message: "rolling_restart_apply: nodes must not be empty".into(),
        });
    }
    Ok(plan(nodes))
}

#[allow(dead_code)]
pub(crate) fn dry_run(nodes: Vec<NodeAddr>) -> ApplyReport {
    let p = plan(nodes);
    let mut steps: Vec<StepResult> = Vec::new();
    let mut step = 0u32;
    loop {
        let r = simulate_step(&p, step);
        let done = r.done;
        steps.push(r.clone());
        if done {
            break;
        }
        step = r.next_step;
    }
    ApplyReport {
        plan: p,
        steps,
        applied: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: &str) -> NodeAddr {
        NodeAddr {
            id: id.into(),
            addr: format!("127.0.0.1:99{id}"),
        }
    }

    #[test]
    fn rolling_restart_apply_rejects_empty_nodes() {
        let result = rolling_restart_apply("127.0.0.1:9999".into(), vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn rolling_restart_apply_returns_plan_with_default_batch() {
        let result =
            rolling_restart_apply("127.0.0.1:9999".into(), vec![n("a"), n("b"), n("c")]).unwrap();
        assert_eq!(result.batch_size, 1);
        assert_eq!(result.health_timeout_ms, 30_000);
        assert_eq!(result.nodes.len(), 3);
    }

    #[test]
    fn dry_run_iterates_all_steps_to_done() {
        let r = dry_run(vec![n("a"), n("b"), n("c"), n("d")]);
        assert!(r.applied);
        assert_eq!(r.plan.nodes.len(), 4);
        assert_eq!(r.steps.len(), 4);
        let last = r.steps.last().unwrap();
        assert!(last.done);
        assert_eq!(last.restarted, vec!["d".to_string()]);
    }
}
