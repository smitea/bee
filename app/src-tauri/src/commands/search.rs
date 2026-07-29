use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub path: Vec<String>,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn pipeline_hit(id: i64, name: &str) -> SearchHit {
    SearchHit {
        kind: "Pipeline".into(),
        id: id.to_string(),
        title: name.into(),
        path: vec!["Pipelines".into()],
    }
}

fn application_hit(id: i64, name: &str) -> SearchHit {
    SearchHit {
        kind: "Application".into(),
        id: id.to_string(),
        title: name.into(),
        path: vec!["Applications".into()],
    }
}

fn datasource_hit(name: &str) -> SearchHit {
    SearchHit {
        kind: "Datasource".into(),
        id: name.into(),
        title: name.into(),
        path: vec!["Datasources".into()],
    }
}

fn dashboard_hit(id: i64, name: &str) -> SearchHit {
    SearchHit {
        kind: "Dashboard".into(),
        id: id.to_string(),
        title: format!("Application {name} · Dashboard"),
        path: vec!["Applications".into(), name.into(), "Dashboard".into()],
    }
}

pub fn collect_hits(conn: &rusqlite::Connection, query: &str) -> Result<Vec<SearchHit>, String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let mut out: Vec<SearchHit> = Vec::new();

    let apps = db::applications::list(conn)?;
    for a in apps {
        if a.name.to_lowercase().contains(&q) {
            out.push(application_hit(a.id, &a.name));
            out.push(dashboard_hit(a.id, &a.name));
        }
    }

    let pipelines = db::pipelines::list(conn)?;
    for p in pipelines {
        if p.name.to_lowercase().contains(&q) || p.dag_json.to_lowercase().contains(&q) {
            out.push(pipeline_hit(p.id, &p.name));
        }
    }

    let datasources = db::datasources::list(conn)?;
    for d in datasources {
        if d.name.to_lowercase().contains(&q) || d.plugin.to_lowercase().contains(&q) {
            out.push(datasource_hit(&d.name));
        }
    }

    Ok(out)
}

#[tauri::command]
pub fn search_local(app: AppHandle, query: String) -> CmdResult<Vec<SearchHit>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    collect_hits(&conn, &query).map_err(CmdError::from)
}

#[tauri::command]
pub fn search_server(query: String) -> CmdResult<Vec<SearchHit>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    log::info!("search_server: graceful fallback for query {q:?} (no admin search-list yet)");
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{applications, datasources, pipelines};
    use crate::db::Database;
    use tempfile::tempdir;

    fn run<F: FnOnce(&rusqlite::Connection)>(f: F) -> Database {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        let conn = db.lock().unwrap();
        f(&conn);
        drop(conn);
        db
    }

    #[test]
    fn collect_hits_empty_query_returns_empty() {
        let db = run(|_| {});
        let conn = db.lock().unwrap();
        assert!(collect_hits(&conn, "").unwrap().is_empty());
        assert!(collect_hits(&conn, "   ").unwrap().is_empty());
    }

    #[test]
    fn collect_hits_returns_application_when_name_matches() {
        let db = run(|conn| {
            applications::create(conn, "alpha-app").unwrap();
            applications::create(conn, "beta-app").unwrap();
        });
        let conn = db.lock().unwrap();
        let hits = collect_hits(&conn, "alpha").unwrap();
        let apps: Vec<_> = hits.iter().filter(|h| h.kind == "Application").collect();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].title, "alpha-app");
    }

    #[test]
    fn collect_hits_returns_dashboard_for_each_matching_application() {
        let db = run(|conn| {
            applications::create(conn, "alpha-app").unwrap();
            applications::create(conn, "alpha-other").unwrap();
        });
        let conn = db.lock().unwrap();
        let hits = collect_hits(&conn, "alpha").unwrap();
        let dashboards: Vec<_> = hits.iter().filter(|h| h.kind == "Dashboard").collect();
        assert_eq!(dashboards.len(), 2);
        for d in dashboards {
            assert_eq!(d.path[0], "Applications");
            assert_eq!(d.path[2], "Dashboard");
        }
    }

    #[test]
    fn collect_hits_returns_pipeline_when_name_or_dag_matches() {
        let db = run(|conn| {
            pipelines::create(conn, "btc_kline", r#"{"k":"alpha"}"#).unwrap();
            pipelines::create(conn, "eth_kline", r#"{"k":"x"}"#).unwrap();
        });
        let conn = db.lock().unwrap();
        let hits = collect_hits(&conn, "alpha").unwrap();
        let pipelines: Vec<_> = hits.iter().filter(|h| h.kind == "Pipeline").collect();
        assert_eq!(pipelines.len(), 1);
        assert_eq!(pipelines[0].title, "btc_kline");
    }

    #[test]
    fn collect_hits_returns_datasource_by_name_or_plugin() {
        let db = run(|conn| {
            datasources::create(conn, "binance", "binance_subscribe", "{}", 0).unwrap();
            datasources::create(conn, "twitter", "twitter_follow", "{}", 0).unwrap();
        });
        let conn = db.lock().unwrap();
        let hits = collect_hits(&conn, "binance").unwrap();
        let ds: Vec<_> = hits.iter().filter(|h| h.kind == "Datasource").collect();
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].title, "binance");
    }

    #[test]
    fn search_local_helper_returns_application_pipeline_datasource_hits() {
        let db = run(|conn| {
            applications::create(conn, "alpha-app").unwrap();
            pipelines::create(conn, "alpha-pipe", "{}").unwrap();
            datasources::create(conn, "alpha-ds", "plug", "{}", 0).unwrap();
        });
        let conn = db.lock().unwrap();
        let hits = collect_hits(&conn, "alpha").unwrap();
        let kinds: Vec<_> = hits.iter().map(|h| h.kind.as_str()).collect();
        assert!(kinds.contains(&"Application"));
        assert!(kinds.contains(&"Pipeline"));
        assert!(kinds.contains(&"Datasource"));
    }

    #[test]
    fn collect_hits_is_case_insensitive() {
        let db = run(|conn| {
            pipelines::create(conn, "BTC_KLINE", "{}").unwrap();
        });
        let conn = db.lock().unwrap();
        let hits = collect_hits(&conn, "btc").unwrap();
        assert!(hits.iter().any(|h| h.kind == "Pipeline"));
    }
}
