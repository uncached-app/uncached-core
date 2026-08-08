use serde_json::json;
use uncached_core::contract::v1::{
    ApiEnvelope, CleanupCapabilities, CleanupResultDto, Platform, ScanFinishedDto, ScanTargetDto,
    TargetFoundDto, TargetSizedDto,
};
use uncached_core::{Category, CoreError, CoreErrorCode};

#[test]
fn scan_target_envelope_has_stable_v1_shape() {
    let value = serde_json::to_value(ApiEnvelope::success(
        Platform::Windows,
        ScanTargetDto {
            path: "C:\\workspace\\node_modules".into(),
            category: Category::NodeModules,
            size_bytes: 123,
            size_pending: false,
            size_has_errors: false,
        },
    ))
    .unwrap();

    assert_eq!(value["apiVersion"], "v1");
    assert_eq!(value["platform"], "windows");
    assert_eq!(value["data"]["sizeBytes"], 123);
    assert!(value["error"].is_null());
}

#[test]
fn failure_envelope_has_null_data_and_public_error_only() {
    let error = CoreError::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "private-native-detail",
    ))
    .public_info();
    let value = serde_json::to_value(ApiEnvelope::<serde_json::Value>::failure(
        Platform::Linux,
        error,
    ))
    .unwrap();

    assert!(value["data"].is_null());
    assert_eq!(value["error"]["code"], "io");
    assert_eq!(value["error"]["message"], "I/O error");
    assert!(!value.to_string().contains("private-native-detail"));
}

#[test]
fn envelope_deserialization_rejects_data_and_error_together() {
    let value = json!({
        "apiVersion": "v1",
        "platform": "windows",
        "data": {"value": 1},
        "error": {
            "code": "io",
            "message": "I/O error",
            "path": null,
            "operation": null
        }
    });

    assert!(serde_json::from_value::<ApiEnvelope<serde_json::Value>>(value).is_err());
}

#[test]
fn envelope_deserialization_rejects_data_and_error_both_missing() {
    let value = json!({
        "apiVersion": "v1",
        "platform": "windows",
        "data": null,
        "error": null
    });

    assert!(serde_json::from_value::<ApiEnvelope<serde_json::Value>>(value).is_err());
}

#[test]
fn envelope_accessors_expose_read_only_contract_state() {
    let success = ApiEnvelope::success(Platform::Windows, json!({"value": 1}));
    assert_eq!(success.api_version(), "v1");
    assert_eq!(success.platform(), &Platform::Windows);
    assert_eq!(success.data(), Some(&json!({"value": 1})));
    assert_eq!(success.error(), None);

    let error = CoreError::Io(std::io::Error::other("private-native-detail")).public_info();
    let failure = ApiEnvelope::<serde_json::Value>::failure(Platform::Linux, error.clone());
    assert_eq!(failure.api_version(), "v1");
    assert_eq!(failure.platform(), &Platform::Linux);
    assert_eq!(failure.data(), None);
    assert_eq!(failure.error(), Some(&error));
}

#[test]
fn transport_dtos_use_camel_case_and_omit_identity() {
    let found = serde_json::to_value(TargetFoundDto {
        scan_id: "scan-1".into(),
        target: ScanTargetDto {
            path: "/workspace/target".into(),
            category: Category::CargoTarget,
            size_bytes: 10,
            size_pending: true,
            size_has_errors: false,
        },
    })
    .unwrap();
    assert_eq!(found["scanId"], "scan-1");
    assert!(found["target"].get("identity").is_none());
    assert_eq!(found["target"]["sizePending"], true);

    let sized = serde_json::to_value(TargetSizedDto {
        scan_id: "scan-1".into(),
        target_id: "target-1".into(),
        size_bytes: 12,
        size_pending: false,
        size_has_errors: true,
        formatted_size: "12 B".into(),
    })
    .unwrap();
    assert_eq!(sized["targetId"], "target-1");
    assert_eq!(sized["sizeHasErrors"], true);
    assert_eq!(sized["formattedSize"], "12 B");
}

#[test]
fn settled_scan_and_cleanup_dtos_have_stable_fields() {
    let scan = serde_json::to_value(ScanFinishedDto {
        scan_id: "scan-1".into(),
        total_targets: 2,
        total_bytes_found: 42,
        duration_ms: 7,
    })
    .unwrap();
    assert_eq!(
        scan,
        json!({
            "scanId": "scan-1",
            "totalTargets": 2,
            "totalBytesFound": 42,
            "durationMs": 7
        })
    );

    let cleanup = serde_json::to_value(CleanupResultDto {
        success: true,
        target_path: r"C:\workspace\target".into(),
        freed_bytes: 42,
        size_has_errors: false,
    })
    .unwrap();
    assert_eq!(
        cleanup,
        json!({
            "success": true,
            "targetPath": r"C:\workspace\target",
            "freedBytes": 42,
            "sizeHasErrors": false
        })
    );
}

#[test]
fn platform_and_capabilities_serialize_as_contract_values() {
    assert_eq!(serde_json::to_value(Platform::Macos).unwrap(), "macos");
    assert_eq!(
        serde_json::to_value(Platform::Unsupported).unwrap(),
        "unsupported"
    );

    let value = serde_json::to_value(CleanupCapabilities {
        permanent_cleanup: true,
        move_to_trash: false,
        secure_identity: true,
    })
    .unwrap();
    assert_eq!(value["secureIdentity"], true);
}

#[test]
fn public_capabilities_reflect_current_secure_backend() {
    let capabilities = uncached_core::cleanup_capabilities();

    let secure_identity_supported = cfg!(windows)
        || cfg!(any(target_os = "linux", target_os = "android"))
        || cfg!(target_os = "macos");

    assert_eq!(capabilities.secure_identity, secure_identity_supported);
    assert_eq!(capabilities.permanent_cleanup, cfg!(windows));
    assert!(!capabilities.move_to_trash);

    let value = serde_json::to_value(capabilities).unwrap();
    assert_eq!(
        value,
        json!({
            "permanentCleanup": cfg!(windows),
            "moveToTrash": false,
            "secureIdentity": secure_identity_supported,
        })
    );
}

#[test]
fn every_core_error_code_is_a_stable_snake_case_string() {
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
fn every_public_error_preserves_contract_shape_without_native_details() {
    let native_detail = "private-native-detail";
    let errors = [
        CoreError::InvalidWorkspace(native_detail.into()),
        CoreError::InvalidTarget(native_detail.into()),
        CoreError::Io(std::io::Error::other(native_detail)),
        CoreError::Trash(native_detail.into()),
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
            operation: "permanent_cleanup".into(),
            source: std::io::Error::other(native_detail),
        },
        CoreError::SecureTargetOpenFailed {
            path: "target".into(),
            source: std::io::Error::other(native_detail),
        },
        CoreError::SecureEnumerationFailed {
            path: "target".into(),
            source: std::io::Error::other(native_detail),
        },
        CoreError::SecureHandleQueryFailed {
            path: "target".into(),
            operation: "identity",
            source: std::io::Error::other(native_detail),
        },
        CoreError::SecureDeletionFailed {
            path: "target".into(),
            source: std::io::Error::other(native_detail),
        },
        CoreError::SecureTrashUnavailable,
    ];

    for error in errors {
        let info = error.public_info();
        let value = serde_json::to_value(ApiEnvelope::<serde_json::Value>::failure(
            Platform::Unsupported,
            info.clone(),
        ))
        .unwrap();

        assert!(serde_json::to_value(info.code).unwrap().is_string());
        assert_eq!(value["data"], serde_json::Value::Null);
        assert!(value["error"].is_object());
        assert!(!value.to_string().contains(native_detail));
    }
}

#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "android",
    target_os = "macos"
)))]
#[test]
fn unsupported_platform_capabilities_are_fail_closed() {
    assert_eq!(
        uncached_core::cleanup_capabilities(),
        CleanupCapabilities {
            permanent_cleanup: false,
            move_to_trash: false,
            secure_identity: false,
        }
    );
}
