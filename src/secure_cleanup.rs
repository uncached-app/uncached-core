use crate::error::{CoreError, CoreResult};
use crate::model::ScanTarget;
use crate::scanner::ProviderSet;
use std::path::Path;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod unix;
#[cfg(windows)]
mod windows;

pub(crate) const NON_MACOS_UNIX_FD_BACKEND_SUPPORTED: bool =
    cfg!(any(target_os = "linux", target_os = "android"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupCapabilities {
    pub permanent: bool,
    pub trash: bool,
}

pub(crate) fn cleanup_capabilities() -> crate::contract::v1::CleanupCapabilities {
    let capabilities = backend_capabilities();
    crate::contract::v1::CleanupCapabilities {
        permanent_cleanup: capabilities.permanent,
        move_to_trash: capabilities.trash,
        secure_identity: cfg!(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            windows
        )),
    }
}

fn backend_capabilities() -> CleanupCapabilities {
    #[cfg(windows)]
    {
        return CleanupCapabilities {
            permanent: true,
            trash: true,
        };
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    {
        return CleanupCapabilities {
            permanent: false,
            trash: false,
        };
    }

    #[allow(unreachable_code)]
    CleanupCapabilities {
        permanent: false,
        trash: false,
    }
}

#[derive(Debug)]
enum BackendTarget {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    Unix(unix::UnixTarget),
    #[cfg(target_os = "macos")]
    Macos(macos::MacosTarget),
    #[cfg(windows)]
    Windows(windows::WindowsTarget),
    Unsupported,
}

pub(crate) struct SecureTarget {
    backend: BackendTarget,
}

impl SecureTarget {
    pub(crate) fn capabilities(&self) -> CleanupCapabilities {
        match &self.backend {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            BackendTarget::Unix(target) => target.capabilities(),
            #[cfg(target_os = "macos")]
            BackendTarget::Macos(target) => target.capabilities(),
            #[cfg(windows)]
            BackendTarget::Windows(target) => target.capabilities(),
            BackendTarget::Unsupported => CleanupCapabilities {
                permanent: false,
                trash: false,
            },
        }
    }

    pub(crate) fn delete_permanently(self) -> CoreResult<()> {
        match self.backend {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            BackendTarget::Unix(target) => target.delete_permanently(),
            #[cfg(target_os = "macos")]
            BackendTarget::Macos(target) => target.delete_permanently(),
            #[cfg(windows)]
            BackendTarget::Windows(target) => target.delete_permanently(),
            BackendTarget::Unsupported => Err(CoreError::SecureOperationUnsupported {
                path: std::path::PathBuf::new(),
                operation: "permanent cleanup".to_owned(),
                source: std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "secure permanent cleanup backend unavailable",
                ),
            }),
        }
    }

    pub(crate) fn move_to_trash(self) -> CoreResult<()> {
        match self.backend {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            BackendTarget::Unix(target) => target.move_to_trash(),
            #[cfg(target_os = "macos")]
            BackendTarget::Macos(target) => target.move_to_trash(),
            #[cfg(windows)]
            BackendTarget::Windows(target) => target.move_to_trash(),
            BackendTarget::Unsupported => Err(CoreError::SecureTrashUnavailable),
        }
    }
}

pub(crate) fn open_secure_target(
    root: &Path,
    _providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<SecureTarget> {
    if target.identity.is_none() {
        return Err(CoreError::TargetIdentityUnavailable {
            path: target.path.clone(),
        });
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        return Ok(SecureTarget {
            backend: BackendTarget::Unix(unix::open_secure_target(root, target)?),
        });
    }

    #[cfg(target_os = "macos")]
    {
        return Ok(SecureTarget {
            backend: BackendTarget::Macos(macos::open_secure_target(root, target)?),
        });
    }

    #[cfg(windows)]
    {
        Ok(SecureTarget {
            backend: BackendTarget::Windows(windows::open_secure_target(root, target)?),
        })
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        windows
    )))]
    {
        let _ = root;
        Ok(SecureTarget {
            backend: BackendTarget::Unsupported,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{open_secure_target, CleanupCapabilities, SecureTarget};
    use crate::error::CoreError;
    use crate::model::{Category, ScanTarget};
    use crate::scanner::ProviderSet;
    use serde_json::json;
    use std::path::PathBuf;

    fn target() -> ScanTarget {
        ScanTarget::new(PathBuf::from("target"), Category::NodeModules, 0).with_identity(Some(
            crate::model::FileIdentity {
                volume_serial: 1,
                file_id: [2; 16],
            },
        ))
    }

    #[test]
    fn unsupported_backend_permanent_cleanup_serializes_production_contract() {
        let secure_target = SecureTarget {
            backend: super::BackendTarget::Unsupported,
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
    fn unsupported_backend_trash_error_intentionally_has_no_operation_contract_field() {
        // Trash-unavailable is a capability error, not an operation-specific failure.
        let secure_target = SecureTarget {
            backend: super::BackendTarget::Unsupported,
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
    }

    #[test]
    fn secure_target_reports_no_capabilities_when_backend_is_unsupported() {
        let secure_target = SecureTarget {
            backend: super::BackendTarget::Unsupported,
        };

        assert_eq!(
            secure_target.capabilities(),
            CleanupCapabilities {
                permanent: false,
                trash: false,
            }
        );
    }

    #[test]
    #[cfg(not(any(unix, windows)))]
    fn secure_target_is_consumed_by_operation() {
        let secure_target = open_secure_target(
            PathBuf::from("root").as_path(),
            &ProviderSet::free(),
            &target(),
        )
        .expect("identity-bearing target should establish contract target");

        let _ = secure_target.delete_permanently();
    }

    #[test]
    fn missing_identity_is_typed_before_secure_open() {
        let target = ScanTarget::new(PathBuf::from("target"), Category::NodeModules, 0);

        assert!(matches!(
            open_secure_target(
                PathBuf::from("root").as_path(),
                &ProviderSet::free(),
                &target
            ),
            Err(CoreError::TargetIdentityUnavailable { .. })
        ));
    }

    #[test]
    fn unix_fd_backend_support_matrix_is_explicit() {
        assert_eq!(
            super::NON_MACOS_UNIX_FD_BACKEND_SUPPORTED,
            cfg!(any(target_os = "linux", target_os = "android"))
        );
    }

    #[cfg(all(
        unix,
        not(target_os = "linux"),
        not(target_os = "android"),
        not(target_os = "macos")
    ))]
    #[test]
    fn unsupported_unix_uses_typed_backend() {
        assert!(!super::NON_MACOS_UNIX_FD_BACKEND_SUPPORTED);
        let secure_target = SecureTarget {
            backend: super::BackendTarget::Unsupported,
        };

        assert!(matches!(
            secure_target.delete_permanently(),
            Err(CoreError::SecureOperationUnsupported { .. })
        ));
    }
}
