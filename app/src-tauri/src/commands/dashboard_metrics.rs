use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct DashboardMetricView {
    pub dashboard_id: i64,
    pub panel_id: String,
    pub pipeline_job_id: Option<i64>,
    pub source_field: String,
    pub widget_kind: String,
    pub chart_config_json: String,
    pub updated_at: i64,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn to_view(m: db::dashboard_metrics::DashboardMetric) -> DashboardMetricView {
    DashboardMetricView {
        dashboard_id: m.dashboard_id,
        panel_id: m.panel_id,
        pipeline_job_id: m.pipeline_job_id,
        source_field: m.source_field,
        widget_kind: m.widget_kind,
        chart_config_json: m.chart_config_json,
        updated_at: m.updated_at,
    }
}

#[tauri::command]
pub fn dashboard_metric_get(
    app: AppHandle,
    dashboard_id: i64,
    panel_id: String,
) -> CmdResult<Option<DashboardMetricView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::dashboard_metrics::get(&conn, dashboard_id, &panel_id)
        .map_err(CmdError::from)
        .map(|opt| opt.map(to_view))
}

#[tauri::command]
pub fn dashboard_metric_list(
    app: AppHandle,
    dashboard_id: i64,
) -> CmdResult<Vec<DashboardMetricView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::dashboard_metrics::list_for_dashboard(&conn, dashboard_id)
        .map_err(CmdError::from)
        .map(|xs| xs.into_iter().map(to_view).collect())
}

#[tauri::command]
pub fn dashboard_metric_save(
    app: AppHandle,
    dashboard_id: i64,
    panel_id: String,
    pipeline_job_id: Option<i64>,
    source_field: String,
    widget_kind: String,
    chart_config_json: String,
) -> CmdResult<DashboardMetricView> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let saved = db::dashboard_metrics::upsert(
        &conn,
        dashboard_id,
        &panel_id,
        pipeline_job_id,
        &source_field,
        &widget_kind,
        &chart_config_json,
    )
    .map_err(CmdError::from)?;
    Ok(to_view(saved))
}

#[tauri::command]
pub fn dashboard_metric_delete(
    app: AppHandle,
    dashboard_id: i64,
    panel_id: String,
) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::dashboard_metrics::delete(&conn, dashboard_id, &panel_id)
        .map_err(CmdError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::tempdir;

    fn run<F: FnOnce(&Database)>(f: F) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        f(&db);
    }

    #[test]
    fn save_then_list_round_trips_via_db_repo() {
        run(|db| {
            let conn = db.lock().unwrap();
            db::dashboard_metrics::upsert(&conn, 1, "p1", Some(7), "price", "line_chart", "{}").unwrap();
            let all = db::dashboard_metrics::list_for_dashboard(&conn, 1).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].source_field, "price");
            assert_eq!(all[0].widget_kind, "line_chart");
        });
    }

    #[test]
    fn to_view_extracts_all_fields() {
        let m = db::dashboard_metrics::DashboardMetric {
            dashboard_id: 1,
            panel_id: "p1".into(),
            pipeline_job_id: Some(99),
            source_field: "x".into(),
            widget_kind: "gauge".into(),
            chart_config_json: "{}".into(),
            updated_at: 7,
        };
        let v = to_view(m);
        assert_eq!(v.dashboard_id, 1);
        assert_eq!(v.panel_id, "p1");
        assert_eq!(v.pipeline_job_id, Some(99));
        assert_eq!(v.source_field, "x");
        assert_eq!(v.widget_kind, "gauge");
        assert_eq!(v.chart_config_json, "{}");
        assert_eq!(v.updated_at, 7);
    }
}