use uncached_core::CoreError;

#[test]
fn unsupported_secure_operation_is_typed() {
    let error = CoreError::SecureOperationUnsupported {
        operation: "permanent cleanup".to_owned(),
    };

    assert!(matches!(
        error,
        CoreError::SecureOperationUnsupported { .. }
    ));
}
