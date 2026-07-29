pub mod cluster;
pub mod connection;
pub mod ping;
pub mod plugins;
pub mod profiles;
pub mod settings;
pub mod tabs;
pub mod applications;
pub mod audit;
pub mod datasources;
pub mod pipelines;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CmdError {
    pub message: String,
}

impl<E: std::fmt::Display> From<E> for CmdError {
    fn from(e: E) -> Self {
        Self { message: e.to_string() }
    }
}

pub type CmdResult<T> = Result<T, CmdError>;

pub(crate) static HANDLE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
