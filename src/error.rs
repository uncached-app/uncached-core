use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid workspace: {0}")]
    InvalidWorkspace(String),
    #[error("invalid scan target: {0}")]
    InvalidTarget(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("trash error: {0}")]
    Trash(String),
    #[error("target is too large for the Recycle Bin")]
    RecycleBinTooLarge,
}

pub type CoreResult<T> = Result<T, CoreError>;
