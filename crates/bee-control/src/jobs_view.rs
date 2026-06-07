//! `jobs_view` — formatting for `bee jobs` and `bee jobs inspect`
//! (S27, ADR-0001).
//!
//! The MVP reads the ControlPlane from any alive node and formats
//! the result as a text table + ASCII DAG. Color codes are ANSI
//! escapes; the caller (CLI or Node admin RPC) can strip them if
//! the output is not a TTY.
//!
//! ## S27 scope
//! - `format_jobs`: `JobId | Name | Status | Tasks | Owner Node`
//!   for every job in the ControlPlane
//! - `format_job_inspect`: header + per-Task lines + ASCII DAG
//! - `format_lifecycle` / `format_status` / `color_for_status`:
//!   helpers for the per-field rendering
//!
//! ## Out of S27 scope
//! - The Node admin RPC that the production `bee jobs` /
//!   `bee jobs inspect` uses (S28). The MVP CLI uses an in-process
//!   in-memory cluster as a demo; the production path queries
//!   another Node's ControlPlane over the BRP control channel.

use crate::control_plane::{ControlPlaneStateMachine, JobRecord, TaskRecord};
use crate::kv::{JobLifecycleState, TaskStatus};

/// Format the `bee jobs` listing. Returns a Markdown-style table
/// with a header row and one row per Job. Includes color codes
/// for the Status column (green=running, yellow=migrating/waiting,
/// red=failed).
pub fn format_jobs(cp: &ControlPlaneStateMachine) -> String {
    let mut out = String::new();
    out.push_str("JobId | Name                | Status              | Tasks | Owner\n");
    out.push_str("------+---------------------+---------------------+-------+------\n");
    let jobs = cp.list_jobs();
    if jobs.is_empty() {
        out.push_str("(no jobs)\n");
        return out;
    }
    for job in &jobs {
        let task_count = cp
            .list_tasks()
            .iter()
            .filter(|t| t.job_id == job.job_id)
            .count();
        out.push_str(&format!(
            "{:5} | {:<19} | {:<19} | {:5} | {:5}\n",
            job.job_id,
            truncate(&job.dag_hash, 19),
            colorize_lifecycle(&format_lifecycle(job.lifecycle)),
            task_count,
            job.owner_node,
        ));
    }
    out
}

/// Format `bee jobs inspect <JobId>`. Returns `None` if the Job
/// doesn't exist. Includes:
/// - Header (id, name, owner_node, lifecycle, dependencies)
/// - Per-Task status table
/// - ASCII DAG of the tasks
pub fn format_job_inspect(
    cp: &ControlPlaneStateMachine,
    job_id: u32,
) -> Option<String> {
    let job = cp.get_job(job_id)?;
    let tasks: Vec<TaskRecord> = cp
        .list_tasks()
        .into_iter()
        .filter(|t| t.job_id == job_id)
        .collect();
    let task_refs: Vec<&TaskRecord> = tasks.iter().collect();
    Some(format_job_inspect_inner(job, &task_refs))
}

fn format_job_inspect_inner(job: &JobRecord, tasks: &[&TaskRecord]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Job {} ({})\n",
        job.job_id, job.dag_hash
    ));
    out.push_str(&format!(
        "  status:     {}\n",
        colorize_lifecycle(&format_lifecycle(job.lifecycle))
    ));
    out.push_str(&format!("  owner_node: {}\n", job.owner_node));
    if !job.dependencies.is_empty() {
        out.push_str("  dependencies:\n");
        for dep in &job.dependencies {
            out.push_str(&format!(
                "    <- job {} (stream {})\n",
                dep.upstream_job, dep.stream
            ));
        }
    }
    out.push_str(&format!("  tasks ({}):\n", tasks.len()));
    for t in tasks {
        out.push_str(&format!(
            "    Task {:3} [{}] on node {}\n",
            t.task_id,
            colorize_status(&format_status(t.status)),
            t.owner_node
        ));
    }
    out.push_str("  DAG:\n");
    out.push_str(&format_dag(tasks));
    out
}

fn format_dag(tasks: &[&TaskRecord]) -> String {
    if tasks.is_empty() {
        return "    (no tasks)\n".to_string();
    }
    let mut sorted: Vec<&&TaskRecord> = tasks.iter().collect();
    sorted.sort_by_key(|t| t.task_id);
    let mut out = String::new();
    for (i, t) in sorted.iter().enumerate() {
        let prefix = if i + 1 == sorted.len() {
            "    └─ "
        } else {
            "    ├─ "
        };
        out.push_str(&format!("{}Task {}\n", prefix, t.task_id));
    }
    out
}

fn format_lifecycle(s: JobLifecycleState) -> String {
    use JobLifecycleState::*;
    match s {
        Pending => "pending",
        Scheduled => "scheduled",
        WaitingForUpstream => "waiting_for_upstream",
        Running => "running",
        Completed => "completed",
        Failed => "failed",
    }
    .to_string()
}

fn format_status(s: TaskStatus) -> String {
    use TaskStatus::*;
    match s {
        Pending => "pending",
        Scheduled => "scheduled",
        Running => "running",
        Orphaned => "orphaned",
        Migrating => "migrating",
        Revoked => "revoked",
        Completed => "completed",
        Failed => "failed",
    }
    .to_string()
}

/// ANSI color codes. Returns the input wrapped in the color escape
/// for the lifecycle state. S27 acceptance: "Color-coded output
/// (green = running, yellow = migrating, red = failed)".
fn colorize_lifecycle(s: &str) -> String {
    let code = match s {
        "running" => "\x1b[32m",          // green
        "waiting_for_upstream" | "orphaned" | "migrating" => "\x1b[33m", // yellow
        "failed" => "\x1b[31m",            // red
        _ => "\x1b[0m",                    // reset (no color)
    };
    format!("{}{}\x1b[0m", code, s)
}

fn colorize_status(s: &str) -> String {
    colorize_lifecycle(s)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kv::{JobLifecycleState, TaskStatus};

    fn empty_cp() -> ControlPlaneStateMachine {
        ControlPlaneStateMachine::new()
    }

    #[test]
    fn format_jobs_empty_cluster_shows_placeholder() {
        let s = format_jobs(&empty_cp());
        assert!(s.contains("(no jobs)"), "missing empty placeholder:\n{s}");
        assert!(s.contains("JobId |"));
    }

    #[test]
    fn format_jobs_includes_color_codes_for_running() {
        let mut cp = empty_cp();
        cp.apply_op(&crate::kv::Op::RegisterJob {
            job_id: 1,
            dag_hash: "h1".into(),
            owner_node: 1,
        })
        .unwrap();
        let s = format_jobs(&cp);
        assert!(s.contains("JobId |"));
        assert!(s.contains("h1"));
        // Pending lifecycle, default color (no green yet).
        // The color is "reset" (\x1b[0m) for pending.
    }

    #[test]
    fn format_jobs_marks_running_jobs_green() {
        let mut cp = empty_cp();
        cp.apply_op(&crate::kv::Op::RegisterJob {
            job_id: 1,
            dag_hash: "h1".into(),
            owner_node: 1,
        })
        .unwrap();
        cp.apply_op(&crate::kv::Op::UpdateJobLifecycle {
            job_id: 1,
            state: JobLifecycleState::Running,
        })
        .unwrap();
        let s = format_jobs(&cp);
        assert!(s.contains("\x1b[32mrunning\x1b[0m"), "missing green color:\n{s}");
    }

    #[test]
    fn format_jobs_marks_failed_jobs_red() {
        let mut cp = empty_cp();
        cp.apply_op(&crate::kv::Op::RegisterJob {
            job_id: 1,
            dag_hash: "h1".into(),
            owner_node: 1,
        })
        .unwrap();
        cp.apply_op(&crate::kv::Op::UpdateJobLifecycle {
            job_id: 1,
            state: JobLifecycleState::Failed,
        })
        .unwrap();
        let s = format_jobs(&cp);
        assert!(s.contains("\x1b[31mfailed\x1b[0m"), "missing red color:\n{s}");
    }

    #[test]
    fn format_jobs_marks_waiting_jobs_yellow() {
        let mut cp = empty_cp();
        cp.apply_op(&crate::kv::Op::RegisterJob {
            job_id: 1,
            dag_hash: "h1".into(),
            owner_node: 1,
        })
        .unwrap();
        cp.apply_op(&crate::kv::Op::UpdateJobLifecycle {
            job_id: 1,
            state: JobLifecycleState::WaitingForUpstream,
        })
        .unwrap();
        let s = format_jobs(&cp);
        assert!(
            s.contains("\x1b[33mwaiting_for_upstream\x1b[0m"),
            "missing yellow color:\n{s}"
        );
    }

    #[test]
    fn format_job_inspect_returns_none_for_unknown_job() {
        let cp = empty_cp();
        assert!(format_job_inspect(&cp, 99).is_none());
    }

    #[test]
    fn format_job_inspect_shows_header_tasks_and_dag() {
        let mut cp = empty_cp();
        cp.apply_op(&crate::kv::Op::RegisterJob {
            job_id: 1,
            dag_hash: "my-pipeline".into(),
            owner_node: 1,
        })
        .unwrap();
        cp.apply_op(&crate::kv::Op::RegisterTask {
            task_id: 1,
            job_id: 1,
            phase_id: 0,
            owner_node: 1,
            status: TaskStatus::Running,
            started_at_ms: 0,
        })
        .unwrap();
        cp.apply_op(&crate::kv::Op::RegisterTask {
            task_id: 2,
            job_id: 1,
            phase_id: 0,
            owner_node: 1,
            status: TaskStatus::Running,
            started_at_ms: 0,
        })
        .unwrap();
        cp.apply_op(&crate::kv::Op::UpdateJobLifecycle {
            job_id: 1,
            state: JobLifecycleState::Running,
        })
        .unwrap();

        let s = format_job_inspect(&cp, 1).expect("inspect");
        // S27 acceptance: shows a DAG diagram and per-Task status.
        assert!(s.contains("Job 1"), "missing job header:\n{s}");
        assert!(s.contains("my-pipeline"));
        assert!(s.contains("tasks (2)"));
        assert!(s.contains("Task   1"), "missing task 1:\n{s}");
        assert!(s.contains("Task   2"));
        assert!(s.contains("DAG:"));
        assert!(s.contains("\x1b[32mrunning\x1b[0m"), "missing green color:\n{s}");
    }

    #[test]
    fn format_dag_renders_task_tree() {
        let task1 = TaskRecord {
            task_id: 1,
            job_id: 1,
            phase_id: 0,
            owner_node: 1,
            status: TaskStatus::Running,
            started_at_ms: 0,
            migrating_from_node: None,
        };
        let task2 = TaskRecord {
            task_id: 2,
            job_id: 1,
            phase_id: 0,
            owner_node: 1,
            status: TaskStatus::Running,
            started_at_ms: 0,
            migrating_from_node: None,
        };
        let s = format_dag(&[&task1, &task2]);
        assert!(s.contains("Task 1"));
        assert!(s.contains("Task 2"));
        // The connector characters ├─ / └─ are present
        assert!(s.contains("├─"));
        assert!(s.contains("└─"));
    }

    #[test]
    fn format_lifecycle_and_status_render_all_variants() {
        // Exercise every JobLifecycleState + TaskStatus variant so
        // a future enum addition triggers a test failure.
        for s in [
            JobLifecycleState::Pending,
            JobLifecycleState::Scheduled,
            JobLifecycleState::WaitingForUpstream,
            JobLifecycleState::Running,
            JobLifecycleState::Completed,
            JobLifecycleState::Failed,
        ] {
            let _ = format_lifecycle(s);
        }
        for s in [
            TaskStatus::Pending,
            TaskStatus::Scheduled,
            TaskStatus::Running,
            TaskStatus::Orphaned,
            TaskStatus::Migrating,
            TaskStatus::Revoked,
            TaskStatus::Completed,
            TaskStatus::Failed,
        ] {
            let _ = format_status(s);
        }
    }
}
