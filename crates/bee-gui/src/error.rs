//! GUI error type (placeholder; full implementation in Task 4).

#[derive(Debug, thiserror::Error)]
pub enum GuiError {
    #[error("I/O error: {0}")]
    Io(String),
}