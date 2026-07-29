use bee_control::raft::AdminRequest;

use crate::commands::{CmdError, CmdResult};
use super::cluster::ensure_handle;

#[tauri::command]
pub async fn ping(addr: String) -> CmdResult<String> {
    let handle = ensure_handle(&addr).await?;
    let _rx = handle.call(AdminRequest::Ping).await.map_err(CmdError::from)?;
    Ok("Pong (queued)".to_string())
}
