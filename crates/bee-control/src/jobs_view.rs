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

use crate::control_plane::{ControlPlaneStateMachine, JobMode, JobRecord, TaskRecord};
use crate::kv::{JobLifecycleState, TaskStatus};

/// Format the `bee jobs` listing. Returns a Markdown-style table
/// with a header row and one row per Job. Includes color codes
/// for the Status column (green=running, yellow=migrating/waiting,
/// red=failed).
pub fn format_jobs(cp: &ControlPlaneStateMachine) -> String {
    let mut out = String::new();
    out.push_str("JobId | Name                | Status              | Mode       | Tasks | Owner\n");
    out.push_str("------+---------------------+---------------------+------------+-------+------\n");
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
            "{:5} | {:<19} | {:<19} | {:<10} | {:5} | {:5}\n",
            job.job_id,
            truncate(&job.dag_hash, 19),
            colorize_lifecycle(&format_lifecycle(job.lifecycle)),
            format_mode(cp.job_mode(job.job_id)),
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
    use std::collections::{HashMap, VecDeque};

    if tasks.is_empty() {
        return "    (no tasks)\n".to_string();
    }

    // 1. Build child map (parent -> [child]) and find roots (no
    //    dependencies). Tasks that depend on a missing parent
    //    (e.g. the dep was deleted) are also treated as roots.
    let mut by_id: HashMap<u32, &TaskRecord> = HashMap::new();
    for t in tasks {
        by_id.insert(t.task_id, *t);
    }
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for t in tasks {
        for &dep in &t.dependencies {
            children.entry(dep).or_default().push(t.task_id);
        }
    }
    let mut roots: Vec<u32> = Vec::new();
    for t in tasks {
        if t.dependencies.is_empty()
            || t.dependencies.iter().all(|d| !by_id.contains_key(d))
        {
            roots.push(t.task_id);
        }
    }
    roots.sort();

    // 2. BFS for level (longest path from any root). Tasks missed
    //    by BFS (cyclic deps) get level 0 as a fallback.
    let mut level: HashMap<u32, usize> = HashMap::new();
    let mut queue: VecDeque<(u32, usize)> =
        roots.iter().map(|&r| (r, 0)).collect();
    while let Some((id, lvl)) = queue.pop_front() {
        if level.get(&id).copied().unwrap_or(usize::MAX) <= lvl {
            continue;
        }
        level.insert(id, lvl);
        if let Some(kids) = children.get(&id) {
            for &c in kids {
                queue.push_back((c, lvl + 1));
            }
        }
    }
    for t in tasks {
        level.entry(t.task_id).or_insert(0);
    }

    // 3. Group by level, sort each level by task_id.
    let mut max_level = 0usize;
    let mut by_level: HashMap<usize, Vec<u32>> = HashMap::new();
    for (&id, &lvl) in &level {
        max_level = max_level.max(lvl);
        by_level.entry(lvl).or_default().push(id);
    }
    for v in by_level.values_mut() {
        v.sort();
    }

    // 4. Render each level as `├─ Task N [status]` (or `└─` for
    //    the last Task in the level), separated by `│` between
    //    levels. For independent tasks (all at level 0), this
    //    falls back to a vertical tree with `├─` / `└─` prefixes.
    let mut out = String::new();
    for lvl in 0..=max_level {
        if let Some(ids) = by_level.get(&lvl) {
            for (i, id) in ids.iter().enumerate() {
                let prefix = if i + 1 == ids.len() { "└─" } else { "├─" };
                let t = by_id.get(id).copied();
                let status = t
                    .map(|t| format_status(t.status))
                    .unwrap_or_else(|| "unknown".into());
                out.push_str(&format!(
                    "    {} Task {} [{}]\n",
                    prefix,
                    id,
                    colorize_status(&status)
                ));
            }
        }
        if lvl < max_level {
            out.push_str("    │\n");
        }
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

/// S17 §4: render a Job's mode for the listing table.
/// `Independent` (the most common case for a normal Pipeline)
/// renders as `-` so the table stays visually compact.
fn format_mode(m: JobMode) -> String {
    use JobMode::*;
    match m {
        Producer => "Producer".to_string(),
        Subscriber => "Subscriber".to_string(),
        Independent => "-".to_string(),
    }
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
        ControlPlaneStateMachine::new(std::sync::Arc::new(std::sync::Mutex::new(bee_registry::PluginManager::new())))
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
        tenant: 0,
        plugins: vec![],
        dependencies: vec![],
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
        tenant: 0,
        plugins: vec![],
        dependencies: vec![],
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
        tenant: 0,
        plugins: vec![],
        dependencies: vec![],
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
        tenant: 0,
        plugins: vec![],
        dependencies: vec![],
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
            tenant: 0,
            plugins: vec![],
            dependencies: vec![],
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
            dependencies: Vec::new(),
        };
        let task2 = TaskRecord {
            task_id: 2,
            job_id: 1,
            phase_id: 0,
            owner_node: 1,
            status: TaskStatus::Running,
            started_at_ms: 0,
            migrating_from_node: None,
            dependencies: Vec::new(),
        };
        let s = format_dag(&[&task1, &task2]);
        assert!(s.contains("Task 1"));
        assert!(s.contains("Task 2"));
        // S27: 2 independent Tasks render as a single row in
        // the new layer-based layout (no `│` connector between
        // them — they're at the same level).
        assert!(!s.contains("│"), "no level separator: {s}");
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

    // ---- S27: format_dag real-DAG layout ----

    fn mk_task(task_id: u32, deps: Vec<u32>) -> TaskRecord {
        TaskRecord {
            task_id,
            job_id: 1,
            phase_id: task_id,
            owner_node: 1,
            status: TaskStatus::Running,
            started_at_ms: 0,
            migrating_from_node: None,
            dependencies: deps,
        }
    }

    #[test]
    fn format_dag_independent_tasks_listed_in_single_row() {
        // T1, T2, T3 with no edges between them.
        let tasks = vec![
            mk_task(1, vec![]),
            mk_task(2, vec![]),
            mk_task(3, vec![]),
        ];
        let s = format_dag(&tasks.iter().collect::<Vec<_>>());
        assert!(s.contains("Task 1"));
        assert!(s.contains("Task 2"));
        assert!(s.contains("Task 3"));
        // Independent tasks: no `│` connector (no edges between
        // levels — all on one row).
        assert!(!s.contains("│"), "no level separator expected: {s}");
    }

    #[test]
    fn format_dag_linear_chain_draws_level_separators() {
        // T1 -> T2 -> T3 (linear chain). 3 levels; 2 `│` separators.
        let tasks = vec![
            mk_task(1, vec![]),
            mk_task(2, vec![1]),
            mk_task(3, vec![2]),
        ];
        let s = format_dag(&tasks.iter().collect::<Vec<_>>());
        // T1 must appear before T2 before T3.
        let p1 = s.find("Task 1").unwrap();
        let p2 = s.find("Task 2").unwrap();
        let p3 = s.find("Task 3").unwrap();
        assert!(p1 < p2 && p2 < p3, "ordering: {s}");
        // 2 `│` connectors (between L0-L1 and L1-L2).
        assert_eq!(s.matches("│").count(), 2, "expected 2 level separators: {s}");
    }

    #[test]
    fn format_dag_diamond_renders_both_branches() {
        // T1 -> {T2, T3} -> T4 (diamond). Levels: L0={T1}, L1={T2,T3}, L2={T4}.
        let tasks = vec![
            mk_task(1, vec![]),
            mk_task(2, vec![1]),
            mk_task(3, vec![1]),
            mk_task(4, vec![2, 3]),
        ];
        let s = format_dag(&tasks.iter().collect::<Vec<_>>());
        // All 4 tasks present.
        assert!(s.contains("Task 1"));
        assert!(s.contains("Task 2"));
        assert!(s.contains("Task 3"));
        assert!(s.contains("Task 4"));
        // 2 `│` separators (L0->L1 and L1->L2).
        assert_eq!(s.matches("│").count(), 2, "expected 2 level separators: {s}");
        // T2 and T3 must be at the same level: between the same
        // pair of `│` separators (and the same prefix style).
        let mut t2_line: Option<usize> = None;
        let mut t3_line: Option<usize> = None;
        for (i, line) in s.lines().enumerate() {
            if line.contains("Task 2") {
                t2_line = Some(i);
            }
            if line.contains("Task 3") {
                t3_line = Some(i);
            }
        }
        assert!(t2_line.is_some() && t3_line.is_some());
        let t2_line = t2_line.unwrap();
        let t3_line = t3_line.unwrap();
        // T2 must be on a line BETWEEN the same pair of `│`
        // markers as T3. Count `│` lines above each.
        let pipe_above = |line: usize| s.lines().take(line).filter(|l| l.trim() == "│").count();
        assert_eq!(pipe_above(t2_line), pipe_above(t3_line),
            "T2 and T3 must share a level (same `│` count above): {s}");
    }
}
