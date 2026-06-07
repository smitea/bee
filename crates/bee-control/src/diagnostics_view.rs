//! `diagnostics_view` — formatting for `bee diagnostics <TaskId>` (S28).
//!
//! The MVP shows the per-Task view: id, status, owner_node, the
//! `migrating_from_node` (S28) if `Migrating`, started_at_ms, and
//! placeholder lines for "metrics from S24" + "recent log lines".
//! Real metrics require a Node admin RPC (S28 follow-up); for the
//! MVP the library output is the source of truth.

use crate::control_plane::ControlPlaneStateMachine;
use crate::kv::TaskStatus;

/// Format `bee diagnostics <TaskId>`. Returns `None` if the Task
/// is not registered.
pub async fn format_task_diagnostics(
    cp: &ControlPlaneStateMachine,
    task_id: u32,
) -> Option<String> {
    let task = cp.get_task(task_id)?;
    let job = cp.get_job(task.job_id);

    let mut out = String::new();
    out.push_str(&format!("Task {}\n", task.task_id));
    out.push_str(&format!("  job_id:       {}\n", task.job_id));
    if let Some(j) = job {
        out.push_str(&format!("  job_name:     {}\n", j.dag_hash));
        out.push_str(&format!("  job_lifecycle: {}\n", format_job_lifecycle(j.lifecycle)));
    }
    out.push_str(&format!("  phase_id:     {}\n", task.phase_id));
    out.push_str(&format!(
        "  status:       {}\n",
        colorize_status(&task.status)
    ));
    out.push_str(&format!("  owner_node:   {}\n", task.owner_node));
    out.push_str(&format!(
        "  started_at_ms: {}\n",
        task.started_at_ms
    ));

    // S28 acceptance: `Migrating` shows source + target + progress.
    if task.status == TaskStatus::Migrating {
        if let Some(src) = task.migrating_from_node {
            out.push_str(&format!("  migrating:    {} -> {} (source -> target)\n", src, task.owner_node));
        } else {
            out.push_str(&format!("  migrating:    -> {} (no source recorded)\n", task.owner_node));
        }
        out.push_str("  progress:     (atomic S12 transition; production wires checkpoint resume in S18+)\n");
    }

    out.push_str("\n  --- metrics (S24) ---\n");
    out.push_str("  events_processed_total:       (requires Node admin RPC; S28 follow-up)\n");
    out.push_str("  processing_latency_p50/p99:   (requires Node admin RPC; S28 follow-up)\n");
    out.push_str("  cpu_seconds_total:            (requires Node admin RPC; S28 follow-up)\n");
    out.push_str("  backpressure_wait_seconds_total: (requires Node admin RPC; S28 follow-up)\n");

    out.push_str("\n  --- recent log lines ---\n");
    out.push_str("  (S28 follow-up: tail the task's local log buffer)\n");

    Some(out)
}

fn colorize_status(s: &TaskStatus) -> String {
    let key = match s {
        TaskStatus::Running => "\x1b[32m",          // green
        TaskStatus::Orphaned | TaskStatus::Migrating => "\x1b[33m", // yellow
        TaskStatus::Failed => "\x1b[31m",            // red
        _ => "\x1b[0m",
    };
    format!("{}{:?}\x1b[0m", key, s)
}

fn format_job_lifecycle(s: crate::kv::JobLifecycleState) -> String {
    use crate::kv::JobLifecycleState::*;
    match s {
        Pending => "pending".to_string(),
        Scheduled => "scheduled".to_string(),
        WaitingForUpstream => "waiting_for_upstream".to_string(),
        Running => "running".to_string(),
        Completed => "completed".to_string(),
        Failed => "failed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_plane::ControlPlaneStateMachine;
    use crate::kv::{Op, TaskStatus};

    fn empty_cp() -> ControlPlaneStateMachine {
        ControlPlaneStateMachine::new()
    }

    #[tokio::test]
    async fn format_diagnostics_returns_none_for_unknown_task() {
        let cp = empty_cp();
        assert!(format_task_diagnostics(&cp, 99).await.is_none());
    }

    #[tokio::test]
    async fn format_diagnostics_shows_basic_task_fields() {
        let mut cp = empty_cp();
        cp.apply_op(&Op::RegisterJob {
            job_id: 1,
            dag_hash: "my-job".into(),
            owner_node: 1,
            tenant: 0,
        })
        .unwrap();
        cp.apply_op(&Op::RegisterTask {
            task_id: 1,
            job_id: 1,
            phase_id: 0,
            owner_node: 1,
            status: TaskStatus::Running,
            started_at_ms: 1_700_000_000_000,
        })
        .unwrap();
        let s = format_task_diagnostics(&cp, 1).await.unwrap();
        assert!(s.contains("Task 1"));
        assert!(s.contains("job_name:     my-job"));
        assert!(s.contains("owner_node:   1"));
        assert!(s.contains("started_at_ms: 1700000000000"));
        // Green for running
        assert!(s.contains("\x1b[32mRunning\x1b[0m"));
    }

    #[tokio::test]
    async fn format_diagnostics_migrating_shows_source_and_target() {
        // S28 acceptance: a `Migrating` Task shows Migrating status,
        // source Node, target Node, progress.
        let mut cp = empty_cp();
        cp.apply_op(&Op::RegisterJob {
            job_id: 1,
            dag_hash: "h".into(),
            owner_node: 1,
            tenant: 0,
        })
        .unwrap();
        cp.apply_op(&Op::RegisterTask {
            task_id: 1,
            job_id: 1,
            phase_id: 0,
            owner_node: 1,
            status: TaskStatus::Orphaned,
            started_at_ms: 0,
        })
        .unwrap();
        // StealTask from node 2 → Migrating with source=1, target=2
        cp.apply_op(&Op::StealTask {
            thief_node: 2,
            task_id: 1,
        })
        .unwrap();
        let s = format_task_diagnostics(&cp, 1).await.unwrap();
        assert!(s.contains("Migrating"));
        assert!(s.contains("owner_node:   2"), "target node = 2");
        assert!(s.contains("1 -> 2"), "source -> target line missing");
        assert!(s.contains("progress:"), "progress placeholder missing");
        // Yellow color for Migrating
        assert!(s.contains("\x1b[33mMigrating\x1b[0m"));
    }
}
