use std::path::PathBuf;
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
    #[error("target identity unavailable: {path}")]
    TargetIdentityUnavailable { path: PathBuf },
    #[error("target identity changed: {path}")]
    TargetIdentityChanged { path: PathBuf },
    #[error("target or ancestor is a reparse point or symlink: {path}")]
    TargetReparsePoint { path: PathBuf },
    #[error("secure operation unsupported: {operation}")]
    SecureOperationUnsupported { operation: String },
    #[error("secure target open failed for {path}: {source}")]
    SecureTargetOpenFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("secure enumeration failed for {path}: {source}")]
    SecureEnumerationFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("secure handle query failed for {path} ({operation}): {source}")]
    SecureHandleQueryFailed {
        path: PathBuf,
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("secure deletion failed for {path}: {source}")]
    SecureDeletionFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("secure Trash is unavailable")]
    SecureTrashUnavailable,
}

pub type CoreResult<T> = Result<T, CoreError>;
