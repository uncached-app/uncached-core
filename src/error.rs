use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// Wire-stable error codes. Serialization and deserialization both come from
/// this derive, so the two directions cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreErrorCode {
    InvalidWorkspace,
    Io,
    ProviderUnavailable,
    TargetMissing,
    TargetNotDirectory,
    TargetNameUnsupported,
    TargetProviderMismatch,
    TargetContextInvalid,
    TargetOutsideWorkspace,
    TargetIgnoredByRule,
    TargetIdentityUnavailable,
    TargetIdentityChanged,
    TargetReparsePoint,
    SecureOperationUnsupported,
    SecureTargetOpenFailed,
    SecureTargetTooLarge,
    SecureTargetTooDeep,
    SecureEnumerationFailed,
    SecureHandleQueryFailed,
    SecureDeletionFailed,
    SecureTrashUnavailable,
    SecureTrashInitializationFailed,
    SecureTrashOperationFailed,
    SecureTrashAborted,
    SecureTrashPostconditionFailed,
    /// A running program holds the target. `operation` carries that
    /// program's name, the one detail the reader needs to act on it.
    TargetInUse,
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
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no provider is registered for the target category")]
    ProviderUnavailable,
    #[error("target does not exist: {path}")]
    TargetMissing { path: PathBuf },
    #[error("target is not a directory: {path}")]
    TargetNotDirectory { path: PathBuf },
    #[error("target name is not representable: {path}")]
    TargetNameUnsupported { path: PathBuf },
    #[error("target name does not match its provider: {path}")]
    TargetProviderMismatch { path: PathBuf },
    #[error("target context does not confirm its provider: {path}")]
    TargetContextInvalid { path: PathBuf },
    #[error("target is outside the safe workspace: {path}")]
    TargetOutsideWorkspace { path: PathBuf },
    #[error("target is excluded by an ignore rule: {path}")]
    TargetIgnoredByRule { path: PathBuf },
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
    #[error("secure target exceeds {limit} entries: {path}")]
    SecureTargetTooLarge { path: PathBuf, limit: usize },
    #[error("secure target exceeds {limit} directory levels: {path}")]
    SecureTargetTooDeep { path: PathBuf, limit: usize },
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
    #[error("target is in use by {program}")]
    TargetInUse { program: String },
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
            Self::Io(_) => (CoreErrorCode::Io, "I/O error", None, None),
            Self::ProviderUnavailable => (
                CoreErrorCode::ProviderUnavailable,
                "no provider is registered for the target category",
                None,
                None,
            ),
            Self::TargetMissing { path } => (
                CoreErrorCode::TargetMissing,
                "target does not exist",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::TargetNotDirectory { path } => (
                CoreErrorCode::TargetNotDirectory,
                "target is not a directory",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::TargetNameUnsupported { path } => (
                CoreErrorCode::TargetNameUnsupported,
                "target name is not representable",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::TargetProviderMismatch { path } => (
                CoreErrorCode::TargetProviderMismatch,
                "target name does not match its provider",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::TargetContextInvalid { path } => (
                CoreErrorCode::TargetContextInvalid,
                "target context does not confirm its provider",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::TargetOutsideWorkspace { path } => (
                CoreErrorCode::TargetOutsideWorkspace,
                "target is outside the safe workspace",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::TargetIgnoredByRule { path } => (
                CoreErrorCode::TargetIgnoredByRule,
                "target is excluded by an ignore rule",
                Some(path.to_string_lossy().into_owned()),
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
            Self::SecureTargetTooLarge { path, .. } => (
                CoreErrorCode::SecureTargetTooLarge,
                "secure target exceeds the supported entry count",
                Some(path.to_string_lossy().into_owned()),
                None,
            ),
            Self::SecureTargetTooDeep { path, .. } => (
                CoreErrorCode::SecureTargetTooDeep,
                "secure target exceeds the supported directory depth",
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
            Self::TargetInUse { program } => (
                CoreErrorCode::TargetInUse,
                "target is in use by a running program",
                None,
                Some(program.clone()),
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

    /// Exhaustive by construction: a new `CoreErrorCode` variant fails to compile
    /// here until its wire string is declared.
    fn wire_name(code: CoreErrorCode) -> &'static str {
        match code {
            CoreErrorCode::InvalidWorkspace => "invalid_workspace",
            CoreErrorCode::Io => "io",
            CoreErrorCode::ProviderUnavailable => "provider_unavailable",
            CoreErrorCode::TargetMissing => "target_missing",
            CoreErrorCode::TargetNotDirectory => "target_not_directory",
            CoreErrorCode::TargetNameUnsupported => "target_name_unsupported",
            CoreErrorCode::TargetProviderMismatch => "target_provider_mismatch",
            CoreErrorCode::TargetContextInvalid => "target_context_invalid",
            CoreErrorCode::TargetOutsideWorkspace => "target_outside_workspace",
            CoreErrorCode::TargetIgnoredByRule => "target_ignored_by_rule",
            CoreErrorCode::TargetIdentityUnavailable => "target_identity_unavailable",
            CoreErrorCode::TargetIdentityChanged => "target_identity_changed",
            CoreErrorCode::TargetReparsePoint => "target_reparse_point",
            CoreErrorCode::SecureOperationUnsupported => "secure_operation_unsupported",
            CoreErrorCode::SecureTargetOpenFailed => "secure_target_open_failed",
            CoreErrorCode::SecureTargetTooLarge => "secure_target_too_large",
            CoreErrorCode::SecureTargetTooDeep => "secure_target_too_deep",
            CoreErrorCode::SecureEnumerationFailed => "secure_enumeration_failed",
            CoreErrorCode::SecureHandleQueryFailed => "secure_handle_query_failed",
            CoreErrorCode::SecureDeletionFailed => "secure_deletion_failed",
            CoreErrorCode::SecureTrashUnavailable => "secure_trash_unavailable",
            CoreErrorCode::SecureTrashInitializationFailed => "secure_trash_initialization_failed",
            CoreErrorCode::SecureTrashOperationFailed => "secure_trash_operation_failed",
            CoreErrorCode::SecureTrashAborted => "secure_trash_aborted",
            CoreErrorCode::SecureTrashPostconditionFailed => "secure_trash_postcondition_failed",
            CoreErrorCode::TargetInUse => "target_in_use",
        }
    }

    const ALL_CODES: [CoreErrorCode; 26] = [
        CoreErrorCode::InvalidWorkspace,
        CoreErrorCode::Io,
        CoreErrorCode::ProviderUnavailable,
        CoreErrorCode::TargetMissing,
        CoreErrorCode::TargetNotDirectory,
        CoreErrorCode::TargetNameUnsupported,
        CoreErrorCode::TargetProviderMismatch,
        CoreErrorCode::TargetContextInvalid,
        CoreErrorCode::TargetOutsideWorkspace,
        CoreErrorCode::TargetIgnoredByRule,
        CoreErrorCode::TargetIdentityUnavailable,
        CoreErrorCode::TargetIdentityChanged,
        CoreErrorCode::TargetReparsePoint,
        CoreErrorCode::SecureOperationUnsupported,
        CoreErrorCode::SecureTargetOpenFailed,
        CoreErrorCode::SecureTargetTooLarge,
        CoreErrorCode::SecureTargetTooDeep,
        CoreErrorCode::SecureEnumerationFailed,
        CoreErrorCode::SecureHandleQueryFailed,
        CoreErrorCode::SecureDeletionFailed,
        CoreErrorCode::SecureTrashUnavailable,
        CoreErrorCode::SecureTrashInitializationFailed,
        CoreErrorCode::SecureTrashOperationFailed,
        CoreErrorCode::SecureTrashAborted,
        CoreErrorCode::SecureTrashPostconditionFailed,
        CoreErrorCode::TargetInUse,
    ];

    fn io_error() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "private-native-detail-7f3c",
        )
    }

    /// Every `CoreError` variant, used by the mapping and sanitization tests.
    fn every_error() -> Vec<CoreError> {
        vec![
            CoreError::InvalidWorkspace("private-workspace-detail".into()),
            CoreError::Io(io_error()),
            CoreError::ProviderUnavailable,
            CoreError::TargetMissing {
                path: "target".into(),
            },
            CoreError::TargetNotDirectory {
                path: "target".into(),
            },
            CoreError::TargetNameUnsupported {
                path: "target".into(),
            },
            CoreError::TargetProviderMismatch {
                path: "target".into(),
            },
            CoreError::TargetContextInvalid {
                path: "target".into(),
            },
            CoreError::TargetOutsideWorkspace {
                path: "target".into(),
            },
            CoreError::TargetIgnoredByRule {
                path: "target".into(),
            },
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
            CoreError::SecureTargetTooLarge {
                path: "target".into(),
                limit: 8,
            },
            CoreError::SecureTargetTooDeep {
                path: "target".into(),
                limit: 4,
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
            CoreError::SecureTrashInitializationFailed { source: io_error() },
            CoreError::SecureTrashOperationFailed {
                path: "target".into(),
                operation: "move_to_trash".into(),
                source: io_error(),
            },
            CoreError::SecureTrashAborted {
                path: "target".into(),
            },
            CoreError::SecureTrashPostconditionFailed {
                path: "target".into(),
            },
            CoreError::TargetInUse {
                program: "Xcode".into(),
            },
        ]
    }

    #[test]
    fn error_codes_round_trip_as_stable_snake_case() {
        for code in ALL_CODES {
            let expected = wire_name(code);
            assert_eq!(serde_json::to_value(code).unwrap(), json!(expected));
            assert_eq!(
                serde_json::from_value::<CoreErrorCode>(json!(expected)).unwrap(),
                code
            );
        }
    }

    #[test]
    fn error_code_table_covers_every_variant_exactly_once() {
        let mut names: Vec<_> = ALL_CODES.into_iter().map(wire_name).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();

        assert_eq!(names.len(), unique);
        assert_eq!(names.len(), ALL_CODES.len());
    }

    #[test]
    fn unknown_error_codes_are_rejected() {
        assert!(serde_json::from_value::<CoreErrorCode>(json!("nope")).is_err());
    }

    #[test]
    fn public_info_maps_every_core_error_variant_to_a_declared_code() {
        let mut seen = Vec::new();
        for error in every_error() {
            let info = error.public_info();
            assert_eq!(
                serde_json::to_value(info.code).unwrap(),
                json!(wire_name(info.code))
            );
            seen.push(info.code);
        }

        for code in ALL_CODES {
            assert!(seen.contains(&code), "no error maps to {code:?}");
        }
    }

    #[test]
    fn public_info_reports_path_for_target_scoped_errors() {
        let cases = [
            (
                CoreError::TargetMissing {
                    path: "target".into(),
                },
                CoreErrorCode::TargetMissing,
                "target does not exist",
            ),
            (
                CoreError::TargetOutsideWorkspace {
                    path: "target".into(),
                },
                CoreErrorCode::TargetOutsideWorkspace,
                "target is outside the safe workspace",
            ),
            (
                CoreError::SecureTargetTooLarge {
                    path: "target".into(),
                    limit: 8,
                },
                CoreErrorCode::SecureTargetTooLarge,
                "secure target exceeds the supported entry count",
            ),
            (
                CoreError::SecureTargetTooDeep {
                    path: "target".into(),
                    limit: 4,
                },
                CoreErrorCode::SecureTargetTooDeep,
                "secure target exceeds the supported directory depth",
            ),
        ];

        for (error, expected_code, expected_message) in cases {
            let info = error.public_info();

            assert_eq!(info.code, expected_code);
            assert_eq!(info.message, expected_message);
            assert_eq!(info.path.as_deref(), Some("target"));
            assert_eq!(info.operation, None);
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
        use std::os::unix::ffi::OsStringExt;

        let error = CoreError::TargetIdentityChanged {
            path: std::path::PathBuf::from(std::ffi::OsString::from_vec(
                b"target\xFFpath".to_vec(),
            )),
        };

        assert_eq!(
            error.public_info().path.as_deref(),
            Some("target\u{fffd}path")
        );
    }

    #[test]
    fn public_info_leaves_generic_context_absent() {
        for error in [
            CoreError::InvalidWorkspace("workspace".into()),
            CoreError::Io(io_error()),
            CoreError::ProviderUnavailable,
            CoreError::SecureTrashUnavailable,
            CoreError::SecureTrashInitializationFailed { source: io_error() },
        ] {
            let info = error.public_info();
            assert_eq!(info.path, None);
            assert_eq!(info.operation, None);
        }
    }

    #[test]
    fn public_info_json_has_exact_shape_and_excludes_native_details() {
        for error in every_error() {
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
            assert!(!json.to_string().contains("private-workspace-detail"));
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
    fn limit_errors_display_their_bound() {
        assert_eq!(
            CoreError::SecureTargetTooLarge {
                path: "target".into(),
                limit: 500_000,
            }
            .to_string(),
            "secure target exceeds 500000 entries: target"
        );
        assert_eq!(
            CoreError::SecureTargetTooDeep {
                path: "target".into(),
                limit: 512,
            }
            .to_string(),
            "secure target exceeds 512 directory levels: target"
        );
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
}
