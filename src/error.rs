use serde::Serialize;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreErrorCode {
    InvalidWorkspace,
    InvalidTarget,
    Io,
    Trash,
    RecycleBinTooLarge,
    TargetIdentityUnavailable,
    TargetIdentityChanged,
    TargetReparsePoint,
    SecureOperationUnsupported,
    SecureTargetOpenFailed,
    SecureEnumerationFailed,
    SecureHandleQueryFailed,
    SecureDeletionFailed,
    SecureTrashUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoreErrorInfo {
    pub code: CoreErrorCode,
    pub message: String,
    pub path: Option<String>,
    pub operation: Option<String>,
}

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
    #[error("secure operation unsupported for {path} ({operation}): {source}")]
    SecureOperationUnsupported {
        path: PathBuf,
        operation: String,
        #[source]
        source: std::io::Error,
    },
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

impl CoreError {
    pub fn public_info(&self) -> CoreErrorInfo {
        let (code, message, path, operation) = match self {
            Self::InvalidWorkspace(_) => (
                CoreErrorCode::InvalidWorkspace,
                "invalid workspace",
                None,
                None,
            ),
            Self::InvalidTarget(_) => (
                CoreErrorCode::InvalidTarget,
                "invalid scan target",
                None,
                None,
            ),
            Self::Io(_) => (CoreErrorCode::Io, "I/O error", None, None),
            Self::Trash(_) => (CoreErrorCode::Trash, "trash error", None, None),
            Self::RecycleBinTooLarge => (
                CoreErrorCode::RecycleBinTooLarge,
                "target is too large for the Recycle Bin",
                None,
                None,
            ),
            Self::TargetIdentityUnavailable { path } => (
                CoreErrorCode::TargetIdentityUnavailable,
                "target identity unavailable",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::TargetIdentityChanged { path } => (
                CoreErrorCode::TargetIdentityChanged,
                "target identity changed",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::TargetReparsePoint { path } => (
                CoreErrorCode::TargetReparsePoint,
                "target or ancestor is a reparse point or symlink",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::SecureOperationUnsupported {
                path, operation, ..
            } => (
                CoreErrorCode::SecureOperationUnsupported,
                "secure operation unsupported",
                Some(path.to_string_lossy().into_owned()),
                Some(operation.clone()),
            ),
            Self::SecureTargetOpenFailed { path, .. } => (
                CoreErrorCode::SecureTargetOpenFailed,
                "secure target open failed",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::SecureEnumerationFailed { path, .. } => (
                CoreErrorCode::SecureEnumerationFailed,
                "secure enumeration failed",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::SecureHandleQueryFailed {
                path, operation, ..
            } => (
                CoreErrorCode::SecureHandleQueryFailed,
                "secure handle query failed",
                Some(path.to_string_lossy().into_owned()),
                Some((*operation).to_owned()),
            ),
            Self::SecureDeletionFailed { path, .. } => (
                CoreErrorCode::SecureDeletionFailed,
                "secure deletion failed",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::SecureTrashUnavailable => (
                CoreErrorCode::SecureTrashUnavailable,
                "secure Trash is unavailable",
                None,
                None,
            ),
        };

        CoreErrorInfo {
            code,
            message: message.to_owned(),
            path,
            operation,
        }
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::error::Error;

    fn io_error() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "private-native-detail-7f3c",
        )
    }

    #[test]
    fn error_codes_serialize_as_stable_snake_case() {
        let cases = [
            (CoreErrorCode::InvalidWorkspace, "invalid_workspace"),
            (CoreErrorCode::InvalidTarget, "invalid_target"),
            (CoreErrorCode::Io, "io"),
            (CoreErrorCode::Trash, "trash"),
            (CoreErrorCode::RecycleBinTooLarge, "recycle_bin_too_large"),
            (
                CoreErrorCode::TargetIdentityUnavailable,
                "target_identity_unavailable",
            ),
            (
                CoreErrorCode::TargetIdentityChanged,
                "target_identity_changed",
            ),
            (CoreErrorCode::TargetReparsePoint, "target_reparse_point"),
            (
                CoreErrorCode::SecureOperationUnsupported,
                "secure_operation_unsupported",
            ),
            (
                CoreErrorCode::SecureTargetOpenFailed,
                "secure_target_open_failed",
            ),
            (
                CoreErrorCode::SecureEnumerationFailed,
                "secure_enumeration_failed",
            ),
            (
                CoreErrorCode::SecureHandleQueryFailed,
                "secure_handle_query_failed",
            ),
            (
                CoreErrorCode::SecureDeletionFailed,
                "secure_deletion_failed",
            ),
            (
                CoreErrorCode::SecureTrashUnavailable,
                "secure_trash_unavailable",
            ),
        ];

        for (code, expected) in cases {
            assert_eq!(serde_json::to_value(code).unwrap(), json!(expected));
        }
    }

    #[test]
    fn public_info_maps_every_core_error_variant() {
        let cases = [
            (
                CoreError::InvalidWorkspace("workspace missing".into()),
                CoreErrorCode::InvalidWorkspace,
            ),
            (
                CoreError::InvalidTarget("target missing".into()),
                CoreErrorCode::InvalidTarget,
            ),
            (CoreError::Io(io_error()), CoreErrorCode::Io),
            (
                CoreError::Trash("trash failed".into()),
                CoreErrorCode::Trash,
            ),
            (
                CoreError::RecycleBinTooLarge,
                CoreErrorCode::RecycleBinTooLarge,
            ),
            (
                CoreError::TargetIdentityUnavailable {
                    path: "target".into(),
                },
                CoreErrorCode::TargetIdentityUnavailable,
            ),
            (
                CoreError::TargetIdentityChanged {
                    path: "target".into(),
                },
                CoreErrorCode::TargetIdentityChanged,
            ),
            (
                CoreError::TargetReparsePoint {
                    path: "target".into(),
                },
                CoreErrorCode::TargetReparsePoint,
            ),
            (
                CoreError::SecureOperationUnsupported {
                    path: "target".into(),
                    operation: "delete".into(),
                    source: io_error(),
                },
                CoreErrorCode::SecureOperationUnsupported,
            ),
            (
                CoreError::SecureTargetOpenFailed {
                    path: "target".into(),
                    source: io_error(),
                },
                CoreErrorCode::SecureTargetOpenFailed,
            ),
            (
                CoreError::SecureEnumerationFailed {
                    path: "target".into(),
                    source: io_error(),
                },
                CoreErrorCode::SecureEnumerationFailed,
            ),
            (
                CoreError::SecureHandleQueryFailed {
                    path: "target".into(),
                    operation: "query",
                    source: io_error(),
                },
                CoreErrorCode::SecureHandleQueryFailed,
            ),
            (
                CoreError::SecureDeletionFailed {
                    path: "target".into(),
                    source: io_error(),
                },
                CoreErrorCode::SecureDeletionFailed,
            ),
            (
                CoreError::SecureTrashUnavailable,
                CoreErrorCode::SecureTrashUnavailable,
            ),
        ];

        for (error, expected_code) in cases {
            assert_eq!(error.public_info().code, expected_code);
        }
    }

    #[test]
    fn public_info_preserves_path_and_operation_context() {
        let error = CoreError::SecureOperationUnsupported {
            path: "target/path".into(),
            operation: "secure_delete".into(),
            source: io_error(),
        };

        let info = error.public_info();

        assert_eq!(info.path.as_deref(), Some("target/path"));
        assert_eq!(info.operation.as_deref(), Some("secure_delete"));
    }

    #[cfg(unix)]
    #[test]
    fn public_info_uses_lossy_path_conversion() {
        let error = CoreError::TargetIdentityChanged {
            path: std::path::PathBuf::from(std::ffi::OsString::from_vec(
                b"target\xFFpath".to_vec(),
            )),
        };

        assert_eq!(error.public_info().path.as_deref(), Some("target�path"));
    }

    #[test]
    fn public_info_leaves_generic_context_absent() {
        for error in [
            CoreError::InvalidWorkspace("workspace".into()),
            CoreError::InvalidTarget("target".into()),
            CoreError::Io(io_error()),
            CoreError::Trash("trash".into()),
            CoreError::RecycleBinTooLarge,
            CoreError::SecureTrashUnavailable,
        ] {
            let info = error.public_info();
            assert_eq!(info.path, None);
            assert_eq!(info.operation, None);
        }
    }

    #[test]
    fn public_info_json_contains_only_public_fields_and_no_native_source() {
        let error = CoreError::SecureTargetOpenFailed {
            path: "target".into(),
            source: io_error(),
        };
        let display = error.to_string();
        let json = serde_json::to_value(error.public_info()).unwrap();
        let json_text = json.to_string();

        assert_eq!(
            json,
            json!({
                "code": "secure_target_open_failed",
                "message": "secure target open failed",
                "path": "target",
                "operation": null,
            })
        );
        assert!(!json.as_object().unwrap().contains_key("source"));
        assert!(!json_text.contains("private-native-detail-7f3c"));
        assert!(!json_text.contains("Permission denied"));
        assert_eq!(
            display,
            "secure target open failed for target: private-native-detail-7f3c"
        );
        assert!(error.source().is_some());
    }

    #[test]
    fn public_info_preserves_display_and_source_behavior() {
        let error = CoreError::Io(io_error());
        let display = error.to_string();
        assert!(error.source().is_some());

        let info = error.public_info();

        assert_eq!(info.message, "I/O error");
        assert!(error.source().is_some());
        assert_eq!(error.to_string(), display);
    }

    #[test]
    fn core_error_info_serializes_as_public_contract() {
        let info = CoreErrorInfo {
            code: CoreErrorCode::SecureOperationUnsupported,
            message: "message".into(),
            path: Some("target".into()),
            operation: Some("delete".into()),
        };

        assert_eq!(
            serde_json::to_value(info).unwrap(),
            json!({
                "code": "secure_operation_unsupported",
                "message": "message",
                "path": "target",
                "operation": "delete",
            })
        );
    }

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
}
