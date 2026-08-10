use crate::error::{CoreError, CoreResult};
use crate::filesystem::file_identity;
use crate::model::{FileIdentity, ScanTarget};
use std::path::Path;

#[cfg(windows)]
mod windows;
#[cfg(all(test, windows))]
pub(crate) use windows::install_shell_item_boundary_hook;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CleanupCapabilities {
    pub(crate) permanent: bool,
    pub(crate) trash: bool,
}

/// The cleanup a caller intends to perform. Capabilities are checked against it
/// before any filesystem work happens, so an unsupported platform refuses
/// without opening handles for the target tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CleanupIntent {
    Permanent,
    Trash,
}

pub(crate) fn cleanup_capabilities() -> crate::contract::v1::CleanupCapabilities {
    let capabilities = backend_capabilities();
    crate::contract::v1::CleanupCapabilities {
        permanent_cleanup: capabilities.permanent,
        move_to_trash: capabilities.trash,
        secure_identity: cfg!(any(unix, windows)),
    }
}

fn backend_capabilities() -> CleanupCapabilities {
    #[cfg(windows)]
    {
        CleanupCapabilities {
            permanent: true,
            trash: windows::shell_backend_available(),
        }
    }

    #[cfg(unix)]
    {
        CleanupCapabilities {
            permanent: true,
            trash: true,
        }
    }

    // Platforms without a native secure cleanup backend remain unavailable.
    #[cfg(not(any(windows, unix)))]
    {
        CleanupCapabilities {
            permanent: false,
            trash: false,
        }
    }
}

#[derive(Debug)]
enum BackendTarget {
    #[cfg(windows)]
    Windows(windows::WindowsTarget),
    #[cfg(unix)]
    Unix {
        path: std::path::PathBuf,
        identity: FileIdentity,
    },
    /// Constructed on platforms without a secure backend, and by the tests that
    /// pin the unsupported-platform contract on every platform.
    #[cfg_attr(windows, allow(dead_code))]
    #[allow(dead_code)]
    Unsupported,
}

pub(crate) struct SecureTarget {
    backend: BackendTarget,
}

impl SecureTarget {
    pub(crate) fn delete_permanently(self) -> CoreResult<()> {
        match self.backend {
            #[cfg(windows)]
            BackendTarget::Windows(target) => target.delete_permanently(),
            #[cfg(unix)]
            BackendTarget::Unix { path, identity } => {
                verify_unix_identity(&path, identity)?;
                std::fs::remove_dir_all(&path)
                    .map_err(|source| CoreError::SecureDeletionFailed { path, source })
            }
            BackendTarget::Unsupported => Err(permanent_cleanup_unsupported()),
        }
    }

    pub(crate) fn move_to_trash(self) -> CoreResult<()> {
        match self.backend {
            #[cfg(windows)]
            BackendTarget::Windows(target) => target.move_to_trash(),
            #[cfg(unix)]
            BackendTarget::Unix { path, identity } => {
                verify_unix_identity(&path, identity)?;
                trash::delete(&path).map_err(|error| CoreError::SecureTrashOperationFailed {
                    path,
                    operation: "move_to_trash".to_owned(),
                    source: std::io::Error::other(error.to_string()),
                })
            }
            BackendTarget::Unsupported => Err(CoreError::SecureTrashUnavailable),
        }
    }
}

#[cfg(unix)]
fn verify_unix_identity(path: &std::path::Path, expected: FileIdentity) -> CoreResult<()> {
    let actual = file_identity(path).ok_or_else(|| CoreError::TargetIdentityUnavailable {
        path: path.to_path_buf(),
    })?;
    if actual != expected {
        return Err(CoreError::TargetIdentityChanged {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn permanent_cleanup_unsupported() -> CoreError {
    CoreError::SecureOperationUnsupported {
        path: std::path::PathBuf::new(),
        operation: "permanent cleanup".to_owned(),
        source: std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "secure permanent cleanup backend unavailable",
        ),
    }
}

pub(crate) fn open_secure_target(
    root: &Path,
    target: &ScanTarget,
    intent: CleanupIntent,
) -> CoreResult<SecureTarget> {
    if target.identity.is_none() {
        return Err(CoreError::TargetIdentityUnavailable {
            path: target.path.clone(),
        });
    }

    let capabilities = backend_capabilities();
    match intent {
        CleanupIntent::Permanent if !capabilities.permanent => {
            return Err(permanent_cleanup_unsupported())
        }
        CleanupIntent::Trash if !capabilities.trash => {
            return Err(CoreError::SecureTrashUnavailable)
        }
        _ => {}
    }

    #[cfg(windows)]
    {
        Ok(SecureTarget {
            backend: BackendTarget::Windows(windows::open_secure_target(root, target, intent)?),
        })
    }

    #[cfg(unix)]
    {
        let _ = root;
        Ok(SecureTarget {
            backend: BackendTarget::Unix {
                path: target.path.clone(),
                identity: target.identity.expect("identity checked above"),
            },
        })
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = root;
        Ok(SecureTarget {
            backend: BackendTarget::Unsupported,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{open_secure_target, BackendTarget, CleanupIntent, SecureTarget};
    use crate::error::CoreError;
    use crate::model::{Category, FileIdentity, ScanTarget};
    use serde_json::json;
    use std::path::PathBuf;

    #[allow(dead_code)]
    fn target() -> ScanTarget {
        ScanTarget::new(PathBuf::from("target"), Category::NodeModules, 0).with_identity(Some(
            FileIdentity {
                volume_serial: 1,
                file_id: [2; 16],
            },
        ))
    }

    #[test]
    fn unsupported_backend_permanent_cleanup_serializes_production_contract() {
        let secure_target = SecureTarget {
            backend: BackendTarget::Unsupported,
        };

        let error = secure_target
            .delete_permanently()
            .expect_err("unsupported backend must reject permanent cleanup");
        assert!(matches!(
            error,
            CoreError::SecureOperationUnsupported { .. }
        ));

        let value = serde_json::to_value(error.public_info()).unwrap();
        assert_eq!(
            value,
            json!({
                "code": "secure_operation_unsupported",
                "message": "secure operation unsupported",
                "path": "",
                "operation": "permanent cleanup",
            })
        );
    }

    #[test]
    fn unsupported_backend_trash_error_preserves_filesystem_without_operation_field() {
        // Trash-unavailable is a capability error, not an operation-specific failure.
        let root = std::env::temp_dir().join(format!(
            "uncached-core-unsupported-trash-{}",
            std::process::id()
        ));
        let target_path = root.join("target");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&target_path).unwrap();
        std::fs::write(target_path.join("content.txt"), b"content").unwrap();
        let secure_target = SecureTarget {
            backend: BackendTarget::Unsupported,
        };
        let error = secure_target
            .move_to_trash()
            .expect_err("unsupported backend must reject Trash cleanup");

        assert!(matches!(error, CoreError::SecureTrashUnavailable));
        let info = error.public_info();
        assert_eq!(
            info.code,
            crate::error::CoreErrorCode::SecureTrashUnavailable
        );
        assert_eq!(info.operation, None);
        assert!(serde_json::to_value(info).unwrap()["operation"].is_null());
        assert_eq!(
            std::fs::read(target_path.join("content.txt")).unwrap(),
            b"content"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_identity_is_typed_before_secure_open() {
        let target = ScanTarget::new(PathBuf::from("target"), Category::NodeModules, 0);

        assert!(matches!(
            open_secure_target(
                PathBuf::from("root").as_path(),
                &target,
                CleanupIntent::Permanent
            ),
            Err(CoreError::TargetIdentityUnavailable { .. })
        ));
    }

    #[test]
    fn capability_gate_matches_reported_capabilities() {
        let capabilities = super::backend_capabilities();
        let public = super::cleanup_capabilities();

        assert_eq!(public.permanent_cleanup, capabilities.permanent);
        assert_eq!(public.move_to_trash, capabilities.trash);
        assert_eq!(public.secure_identity, cfg!(any(unix, windows)));
    }

    #[test]
    #[cfg(not(any(windows, unix)))]
    fn unsupported_platform_refuses_before_touching_the_filesystem() {
        // The target path does not exist: reaching the filesystem would fail with
        // an open error instead of the typed capability errors asserted here.
        let missing = PathBuf::from("uncached-core-nonexistent-root");

        assert!(matches!(
            open_secure_target(&missing, &target(), CleanupIntent::Permanent),
            Err(CoreError::SecureOperationUnsupported { .. })
        ));
        assert!(matches!(
            open_secure_target(&missing, &target(), CleanupIntent::Trash),
            Err(CoreError::SecureTrashUnavailable)
        ));
    }

    #[test]
    #[cfg(windows)]
    fn windows_permanent_intent_passes_the_capability_gate() {
        // Windows always reports permanent cleanup, so the gate must let the open
        // proceed and fail on the missing target rather than on capabilities.
        let missing = PathBuf::from("uncached-core-nonexistent-root");

        assert!(matches!(
            open_secure_target(&missing, &target(), CleanupIntent::Permanent),
            Err(CoreError::SecureTargetOpenFailed { .. })
        ));
    }
}
