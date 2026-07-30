use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardMetric {
    pub dashboard_id: i64,
    pub panel_id: String,
    pub pipeline_job_id: Option<i64>,
    pub source_field: String,
    pub widget_kind: String,
    pub chart_config_json: String,
    pub updated_at: i64,
}

pub fn get(
    conn: &Connection,
    dashboard_id: i64,
    panel_id: &str,
) -> Result<Option<DashboardMetric>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT dashboard_id, panel_id, pipeline_job_id, source_field, widget_kind, chart_config_json, updated_at
             FROM dashboard_metrics WHERE dashboard_id = ? AND panel_id = ?",
        )
        .map_err(|e| format!("dashboard_metrics.get prepare: {e}"))?;
    let mut rows = stmt
        .query_map(params![dashboard_id, panel_id], |row| {
            Ok(DashboardMetric {
                dashboard_id: row.get(0)?,
                panel_id: row.get(1)?,
                pipeline_job_id: row.get(2)?,
                source_field: row.get(3)?,
                widget_kind: row.get(4)?,
                chart_config_json: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("dashboard_metrics.get query: {e}"))?;
    match rows.next() {
        Some(Ok(p)) => Ok(Some(p)),
        Some(Err(e)) => Err(format!("dashboard_metrics.get next: {e}")),
        None => Ok(None),
    }
}

pub fn list_for_dashboard(
    conn: &Connection,
    dashboard_id: i64,
) -> Result<Vec<DashboardMetric>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT dashboard_id, panel_id, pipeline_job_id, source_field, widget_kind, chart_config_json, updated_at
             FROM dashboard_metrics WHERE dashboard_id = ? ORDER BY panel_id",
        )
        .map_err(|e| format!("dashboard_metrics.list_for_dashboard prepare: {e}"))?;
    let rows = stmt
        .query_map(params![dashboard_id], |row| {
            Ok(DashboardMetric {
                dashboard_id: row.get(0)?,
                panel_id: row.get(1)?,
                pipeline_job_id: row.get(2)?,
                source_field: row.get(3)?,
                widget_kind: row.get(4)?,
                chart_config_json: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("dashboard_metrics.list_for_dashboard query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("dashboard_metrics.list_for_dashboard collect: {e}"))
}

pub fn upsert(
    conn: &Connection,
    dashboard_id: i64,
    panel_id: &str,
    pipeline_job_id: Option<i64>,
    source_field: &str,
    widget_kind: &str,
    chart_config_json: &str,
) -> Result<DashboardMetric, String> {
    let now = now_secs();
    conn.execute(
        "INSERT INTO dashboard_metrics (dashboard_id, panel_id, pipeline_job_id, source_field, widget_kind, chart_config_json, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(dashboard_id, panel_id) DO UPDATE SET
            pipeline_job_id = excluded.pipeline_job_id,
            source_field = excluded.source_field,
            widget_kind = excluded.widget_kind,
            chart_config_json = excluded.chart_config_json,
            updated_at = excluded.updated_at",
        params![dashboard_id, panel_id, pipeline_job_id, source_field, widget_kind, chart_config_json, now],
    )
    .map_err(|e| format!("dashboard_metrics.upsert: {e}"))?;
    Ok(DashboardMetric {
        dashboard_id,
        panel_id: panel_id.to_string(),
        pipeline_job_id,
        source_field: source_field.to_string(),
        widget_kind: widget_kind.to_string(),
        chart_config_json: chart_config_json.to_string(),
        updated_at: now,
    })
}

pub fn delete(
    conn: &Connection,
    dashboard_id: i64,
    panel_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM dashboard_metrics WHERE dashboard_id = ? AND panel_id = ?",
        params![dashboard_id, panel_id],
    )
    .map_err(|e| format!("dashboard_metrics.delete({dashboard_id},{panel_id}): {e}"))
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::tempdir;

    fn run<F: FnOnce(&Connection)>(f: F) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        let conn = db.lock().unwrap();
        f(&conn);
    }

    #[test]
    fn get_returns_none_when_unset() {
        run(|conn| {
            assert!(get(conn, 1, "p1").unwrap().is_none());
        });
    }

    #[test]
    fn upsert_then_get_round_trips() {
        run(|conn| {
            let saved = upsert(conn, 1, "p1", Some(42), "price", "line_chart", r#"{"color":"red"}"#).unwrap();
            assert_eq!(saved.dashboard_id, 1);
            assert_eq!(saved.panel_id, "p1");
            assert_eq!(saved.pipeline_job_id, Some(42));
            assert_eq!(saved.source_field, "price");
            assert_eq!(saved.widget_kind, "line_chart");
            let fetched = get(conn, 1, "p1").unwrap().unwrap();
            assert_eq!(fetched, saved);
        });
    }

    #[test]
    fn upsert_overwrites_previous_metric_for_same_panel() {
        run(|conn| {
            upsert(conn, 1, "p1", Some(1), "x", "gauge", "{}").unwrap();
            let updated = upsert(conn, 1, "p1", Some(2), "y", "bar_chart", r#"{"x":1}"#).unwrap();
            assert_eq!(updated.pipeline_job_id, Some(2));
            assert_eq!(updated.source_field, "y");
            assert_eq!(updated.widget_kind, "bar_chart");
            let all = list_for_dashboard(conn, 1).unwrap();
            assert_eq!(all.len(), 1);
        });
    }

    #[test]
    fn list_for_dashboard_filters_by_dashboard_id() {
        run(|conn| {
            upsert(conn, 1, "p1", None, "x", "line", "{}").unwrap();
            upsert(conn, 1, "p2", None, "y", "gauge", "{}").unwrap();
            upsert(conn, 2, "p9", None, "z", "stat", "{}").unwrap();
            let xs = list_for_dashboard(conn, 1).unwrap();
            assert_eq!(xs.len(), 2);
            let ys = list_for_dashboard(conn, 2).unwrap();
            assert_eq!(ys.len(), 1);
            assert_eq!(ys[0].panel_id, "p9");
        });
    }

    #[test]
    fn delete_removes_metric_for_panel() {
        run(|conn| {
            upsert(conn, 1, "p1", None, "x", "line", "{}").unwrap();
            delete(conn, 1, "p1").unwrap();
            assert!(get(conn, 1, "p1").unwrap().is_none());
        });
    }

    #[test]
    fn delete_unknown_panel_is_noop_not_error() {
        run(|conn| {
            delete(conn, 1, "missing").unwrap();
        });
    }
}