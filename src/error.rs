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
    SecureTrashInitializationFailed,
    SecureTrashOperationFailed,
    SecureTrashAborted,
    SecureTrashPostconditionFailed,
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
    #[error("secure Trash initialization failed: {source}")]
    SecureTrashInitializationFailed {
        #[source]
        source: std::io::Error,
    },
    #[error("secure Trash operation failed for {path} ({operation}): {source}")]
    SecureTrashOperationFailed {
        path: PathBuf,
        operation: String,
        #[source]
        source: std::io::Error,
    },
    #[error("secure Trash operation was aborted for {path}")]
    SecureTrashAborted { path: PathBuf },
    #[error("secure Trash postcondition failed for {path}")]
    SecureTrashPostconditionFailed { path: PathBuf },
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
            Self::SecureTrashInitializationFailed { .. } => (
                CoreErrorCode::SecureTrashInitializationFailed,
                "secure Trash initialization failed",
                None,
                None,
            ),
            Self::SecureTrashOperationFailed {
                path, operation, ..
            } => (
                CoreErrorCode::SecureTrashOperationFailed,
                "secure Trash operation failed",
                Some(path.to_string_lossy().into_owned()),
                Some(operation.clone()),
            ),
            Self::SecureTrashAborted { path } => (
                CoreErrorCode::SecureTrashAborted,
                "secure Trash operation was aborted",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::SecureTrashPostconditionFailed { path } => (
                CoreErrorCode::SecureTrashPostconditionFailed,
                "secure Trash postcondition failed",
                Some(path.to_string_lossy().into_owned()),
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
            (
                CoreErrorCode::SecureTrashInitializationFailed,
                "secure_trash_initialization_failed",
            ),
            (
                CoreErrorCode::SecureTrashOperationFailed,
                "secure_trash_operation_failed",
            ),
            (CoreErrorCode::SecureTrashAborted, "secure_trash_aborted"),
            (
                CoreErrorCode::SecureTrashPostconditionFailed,
                "secure_trash_postcondition_failed",
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
                "invalid workspace",
                None,
                None,
            ),
            (
                CoreError::InvalidTarget("target missing".into()),
                CoreErrorCode::InvalidTarget,
                "invalid scan target",
                None,
                None,
            ),
            (
                CoreError::Io(io_error()),
                CoreErrorCode::Io,
                "I/O error",
                None,
                None,
            ),
            (
                CoreError::Trash("trash failed".into()),
                CoreErrorCode::Trash,
                "trash error",
                None,
                None,
            ),
            (
                CoreError::RecycleBinTooLarge,
                CoreErrorCode::RecycleBinTooLarge,
                "target is too large for the Recycle Bin",
                None,
                None,
            ),
            (
                CoreError::TargetIdentityUnavailable {
                    path: "target".into(),
                },
                CoreErrorCode::TargetIdentityUnavailable,
                "target identity unavailable",
                Some("target"),
                None,
            ),
            (
                CoreError::TargetIdentityChanged {
                    path: "target".into(),
                },
                CoreErrorCode::TargetIdentityChanged,
                "target identity changed",
                Some("target"),
                None,
            ),
            (
                CoreError::TargetReparsePoint {
                    path: "target".into(),
                },
                CoreErrorCode::TargetReparsePoint,
                "target or ancestor is a reparse point or symlink",
                Some("target"),
                None,
            ),
            (
                CoreError::SecureOperationUnsupported {
                    path: "target".into(),
                    operation: "delete".into(),
                    source: io_error(),
                },
                CoreErrorCode::SecureOperationUnsupported,
                "secure operation unsupported",
                Some("target"),
                Some("delete"),
            ),
            (
                CoreError::SecureTargetOpenFailed {
                    path: "target".into(),
                    source: io_error(),
                },
                CoreErrorCode::SecureTargetOpenFailed,
                "secure target open failed",
                Some("target"),
                None,
            ),
            (
                CoreError::SecureEnumerationFailed {
                    path: "target".into(),
                    source: io_error(),
                },
                CoreErrorCode::SecureEnumerationFailed,
                "secure enumeration failed",
                Some("target"),
                None,
            ),
            (
                CoreError::SecureHandleQueryFailed {
                    path: "target".into(),
                    operation: "query",
                    source: io_error(),
                },
                CoreErrorCode::SecureHandleQueryFailed,
                "secure handle query failed",
                Some("target"),
                Some("query"),
            ),
            (
                CoreError::SecureDeletionFailed {
                    path: "target".into(),
                    source: io_error(),
                },
                CoreErrorCode::SecureDeletionFailed,
                "secure deletion failed",
                Some("target"),
                None,
            ),
            (
                CoreError::SecureTrashUnavailable,
                CoreErrorCode::SecureTrashUnavailable,
                "secure Trash is unavailable",
                None,
                None,
            ),
            (
                CoreError::SecureTrashInitializationFailed { source: io_error() },
                CoreErrorCode::SecureTrashInitializationFailed,
                "secure Trash initialization failed",
                None,
                None,
            ),
            (
                CoreError::SecureTrashOperationFailed {
                    path: "target".into(),
                    operation: "move_to_trash".into(),
                    source: io_error(),
                },
                CoreErrorCode::SecureTrashOperationFailed,
                "secure Trash operation failed",
                Some("target"),
                Some("move_to_trash"),
            ),
            (
                CoreError::SecureTrashAborted {
                    path: "target".into(),
                },
                CoreErrorCode::SecureTrashAborted,
                "secure Trash operation was aborted",
                Some("target"),
                None,
            ),
            (
                CoreError::SecureTrashPostconditionFailed {
                    path: "target".into(),
                },
                CoreErrorCode::SecureTrashPostconditionFailed,
                "secure Trash postcondition failed",
                Some("target"),
                None,
            ),
        ];

        for (error, expected_code, expected_message, expected_path, expected_operation) in cases {
            let info = error.public_info();

            assert_eq!(info.code, expected_code);
            assert_eq!(info.message, expected_message);
            assert_eq!(info.path.as_deref(), expected_path);
            assert_eq!(info.operation.as_deref(), expected_operation);
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
    fn public_info_json_has_exact_shape_and_excludes_native_details() {
        let errors = [
            CoreError::InvalidWorkspace("private-workspace-detail".into()),
            CoreError::InvalidTarget("private-target-detail".into()),
            CoreError::Io(io_error()),
            CoreError::Trash("private-trash-detail".into()),
            CoreError::RecycleBinTooLarge,
            CoreError::TargetIdentityUnavailable {
                path: "target".into(),
            },
            CoreError::TargetIdentityChanged {
                path: "target".into(),
            },
            CoreError::TargetReparsePoint {
                path: "target".into(),
            },
            CoreError::SecureOperationUnsupported {
                path: "target".into(),
                operation: "delete".into(),
                source: io_error(),
            },
            CoreError::SecureTargetOpenFailed {
                path: "target".into(),
                source: io_error(),
            },
            CoreError::SecureEnumerationFailed {
                path: "target".into(),
                source: io_error(),
            },
            CoreError::SecureHandleQueryFailed {
                path: "target".into(),
                operation: "query",
                source: io_error(),
            },
            CoreError::SecureDeletionFailed {
                path: "target".into(),
                source: io_error(),
            },
            CoreError::SecureTrashUnavailable,
        ];

        for error in errors {
            let display = error.to_string();
            let json = serde_json::to_value(error.public_info()).unwrap();
            let object = json.as_object().unwrap();
            let round_trip: serde_json::Value = serde_json::from_str(&json.to_string()).unwrap();

            assert_eq!(object.len(), 4);
            assert!(object.contains_key("code"));
            assert!(object.contains_key("message"));
            assert!(object.contains_key("path"));
            assert!(object.contains_key("operation"));
            assert_eq!(round_trip, json);
            assert!(!json.to_string().contains("source"));
            assert!(!json.to_string().contains("private-native-detail-7f3c"));
            assert!(!json.to_string().contains("Permission denied"));

            if matches!(
                error,
                CoreError::Io(_) | CoreError::SecureTargetOpenFailed { .. }
            ) {
                assert!(display.contains("private-native-detail-7f3c"));
                assert!(error.source().is_some());
            }
        }
    }

    #[test]
    fn public_info_preserves_display_and_source_behavior() {
        let error = CoreError::Io(io_error());
        let display_before = error.to_string();
        let source_before = error.source().unwrap().to_string();
        assert!(error.source().is_some());

        let info = error.public_info();

        assert_eq!(info.message, "I/O error");
        assert_eq!(display_before, "I/O error: private-native-detail-7f3c");
        assert_eq!(source_before, "private-native-detail-7f3c");
        assert_eq!(error.to_string(), "I/O error: private-native-detail-7f3c");
        assert_eq!(
            error.source().unwrap().to_string(),
            "private-native-detail-7f3c"
        );
    }

    #[test]
    fn secure_error_display_and_source_behavior_are_explicit() {
        let cases = [
            (
                CoreError::SecureOperationUnsupported {
                    path: "target".into(),
                    operation: "delete".into(),
                    source: io_error(),
                },
                "secure operation unsupported for target (delete): private-native-detail-7f3c",
            ),
            (
                CoreError::SecureTargetOpenFailed {
                    path: "target".into(),
                    source: io_error(),
                },
                "secure target open failed for target: private-native-detail-7f3c",
            ),
            (
                CoreError::SecureEnumerationFailed {
                    path: "target".into(),
                    source: io_error(),
                },
                "secure enumeration failed for target: private-native-detail-7f3c",
            ),
            (
                CoreError::SecureHandleQueryFailed {
                    path: "target".into(),
                    operation: "query",
                    source: io_error(),
                },
                "secure handle query failed for target (query): private-native-detail-7f3c",
            ),
            (
                CoreError::SecureDeletionFailed {
                    path: "target".into(),
                    source: io_error(),
                },
                "secure deletion failed for target: private-native-detail-7f3c",
            ),
        ];

        for (error, expected_display) in cases {
            assert_eq!(error.to_string(), expected_display);
            assert_eq!(
                error.source().unwrap().to_string(),
                "private-native-detail-7f3c"
            );
        }
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
