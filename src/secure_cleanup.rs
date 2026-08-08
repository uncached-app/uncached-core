use crate::error::{CoreError, CoreResult};
use crate::model::ScanTarget;
use crate::scanner::ProviderSet;
use std::path::Path;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod unix;

pub(crate) const NON_MACOS_UNIX_FD_BACKEND_SUPPORTED: bool =
    cfg!(any(target_os = "linux", target_os = "android"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupCapabilities {
    pub permanent: bool,
    pub trash: bool,
}

#[derive(Debug)]
enum BackendTarget {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    Unix(unix::UnixTarget),
    #[cfg(target_os = "macos")]
    Macos(macos::MacosTarget),
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
            BackendTarget::Unsupported => Err(CoreError::SecureOperationUnsupported {
                operation: "permanent cleanup".to_owned(),
            }),
        }
    }

    pub(crate) fn move_to_trash(self) -> CoreResult<()> {
        match self.backend {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            BackendTarget::Unix(target) => target.move_to_trash(),
            #[cfg(target_os = "macos")]
            BackendTarget::Macos(target) => target.move_to_trash(),
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

    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
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
    fn unsupported_secure_operations_return_typed_errors() {
        let secure_target = SecureTarget {
            backend: super::BackendTarget::Unsupported,
        };

        assert!(matches!(
            secure_target.delete_permanently(),
            Err(CoreError::SecureOperationUnsupported { .. })
        ));

        let secure_target = SecureTarget {
            backend: super::BackendTarget::Unsupported,
        };
        assert!(matches!(
            secure_target.move_to_trash(),
            Err(CoreError::SecureTrashUnavailable)
        ));
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
    #[cfg(not(unix))]
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
