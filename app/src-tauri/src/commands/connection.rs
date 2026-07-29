use std::time::Duration;

use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::commands::HANDLE_LOCK;
use crate::connection::{self, ConnectionHandle};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum ConnStatus {
    Connected,
    Connecting,
    Disconnected,
    Error { reason: String },
}

#[derive(Debug, Serialize, Clone)]
pub struct StateView {
    pub addr: String,
    pub status: ConnStatus,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn view(handle: &ConnectionHandle) -> StateView {
    use crate::connection::ConnectionState as S;
    let status = match handle.state() {
        S::Connected => ConnStatus::Connected,
        S::Connecting => ConnStatus::Connecting,
        S::Disconnected => ConnStatus::Disconnected,
        S::Error(reason) => ConnStatus::Error { reason },
    };
    StateView { addr: handle.addr().to_string(), status }
}

#[tauri::command]
pub fn get_default_addr() -> String {
    std::env::var("BEE_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:9999".to_string())
}

#[tauri::command]
pub async fn set_addr(app: AppHandle, addr: String) -> CmdResult<StateView> {
    let _guard = HANDLE_LOCK.lock().await;
    let parsed = connection::addr_parse(&addr).map_err(CmdError::from)?;
    {
        let db = db_handle(&app)?;
        let conn = db.lock().map_err(CmdError::from)?;
        db::settings::put(&conn, "addr", &addr).map_err(CmdError::from)?;
    }
    let handle = connection::ensure_bundle(parsed);
    Ok(view(&handle))
}

#[tauri::command]
pub async fn test_connection(addr: String) -> CmdResult<StateView> {
    let parsed = connection::addr_parse(&addr).map_err(CmdError::from)?;
    let outcome = tokio::time::timeout(
        Duration::from_secs(1),
        bee_control::raft::admin_client::AdminClient::connect(parsed),
    ).await;
    let status = match outcome {
        Ok(Ok(_client)) => ConnStatus::Connected,
        Ok(Err(e)) => ConnStatus::Error { reason: format!("{e}") },
        Err(_) => ConnStatus::Error { reason: "timeout after 1s".into() },
    };
    Ok(StateView { addr: parsed.to_string(), status })
}

#[tauri::command]
pub fn conn_state(addr: String) -> CmdResult<StateView> {
    let parsed = connection::addr_parse(&addr).map_err(CmdError::from)?;
    let handle = connection::with_handle(|h| Ok(h.clone())).map_err(CmdError::from)?;
    let _ = parsed;
    Ok(view(&handle))
}
