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
        target_id: "target-1".into(),
        freed_bytes: 42,
        size_has_errors: false,
    })
    .unwrap();
    assert_eq!(
        cleanup,
        json!({
            "targetId": "target-1",
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
        permanent: true,
        trash: false,
        secure_identity: true,
    })
    .unwrap();
    assert_eq!(value["secureIdentity"], true);
}

#[test]
fn public_capabilities_reflect_current_secure_backend() {
    let capabilities = uncached_core::cleanup_capabilities();

    assert!(capabilities.secure_identity);
    assert!(capabilities.permanent);
    assert!(!capabilities.trash);
}

#[test]
fn core_error_codes_remain_snake_case() {
    assert_eq!(
        serde_json::to_value(CoreErrorCode::SecureTrashUnavailable).unwrap(),
        "secure_trash_unavailable"
    );
}
