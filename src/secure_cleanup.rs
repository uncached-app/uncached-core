use crate::error::{CoreError, CoreResult};
#[cfg(all(unix, not(target_os = "android")))]
use crate::filesystem::file_identity;
#[cfg(all(unix, not(target_os = "android")))]
use crate::model::FileIdentity;
use crate::model::ScanTarget;
use std::path::Path;

#[cfg(windows)]
mod windows;
#[cfg(all(test, windows))]
pub(crate) use windows::install_shell_item_boundary_hook;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CleanupCapabilities {
    pub(crate) permanent: bool,
    pub(crate) trash: bool,
    /// Whether a stable filesystem identity can be read and checked on this
    /// platform. Not a cleanup backend itself, but it travels with them: it is
    /// true exactly where a secure backend exists.
    pub(crate) secure_identity: bool,
}

/// The cleanup a caller intends to perform. Capabilities are checked against it
/// before any filesystem work happens, so an unsupported platform refuses
/// without opening handles for the target tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CleanupIntent {
    Permanent,
    Trash,
}

pub(crate) fn backend_capabilities() -> CleanupCapabilities {
    #[cfg(windows)]
    {
        CleanupCapabilities {
            permanent: true,
            trash: windows::shell_backend_available(),
            secure_identity: true,
        }
    }

    #[cfg(all(unix, not(target_os = "android")))]
    {
        CleanupCapabilities {
            permanent: true,
            trash: true,
            secure_identity: true,
        }
    }

    // Platforms without a native secure cleanup backend remain unavailable.
    #[cfg(not(any(windows, all(unix, not(target_os = "android")))))]
    {
        CleanupCapabilities {
            permanent: false,
            trash: false,
            secure_identity: false,
        }
    }
}

#[derive(Debug)]
enum BackendTarget {
    #[cfg(windows)]
    Windows(windows::WindowsTarget),
    #[cfg(all(unix, not(target_os = "android")))]
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
            #[cfg(all(unix, not(target_os = "android")))]
            BackendTarget::Unix { path, identity } => delete_unix_target(&path, identity),
            BackendTarget::Unsupported => Err(permanent_cleanup_unsupported()),
        }
    }

    pub(crate) fn move_to_trash(self) -> CoreResult<()> {
        match self.backend {
            #[cfg(windows)]
            BackendTarget::Windows(target) => target.move_to_trash(),
            #[cfg(all(unix, not(target_os = "android")))]
            BackendTarget::Unix { path, identity } => {
                // The last check this crate can make. What follows is the `trash`
                // crate's own path-based move: it canonicalizes the *parent* and
                // keeps the final name (so a symlink swapped in for the target
                // is moved as a link, not followed), then renames that name into
                // the Trash. Between this check and that rename someone who can
                // write to the parent directory could swap the target for
                // another directory, and the other one would be moved. The
                // destination is the Trash, so the result is recoverable, and
                // closing the window would mean reimplementing the freedesktop
                // spec on file descriptors, which cannot be verified on macOS
                // from here. Permanent deletion has no such window: it works
                // through a pinned descriptor (`delete_unix_target`).
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

#[cfg(all(unix, not(target_os = "android")))]
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

#[cfg(all(unix, not(target_os = "android")))]
fn delete_unix_target(path: &Path, expected: FileIdentity) -> CoreResult<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| CoreError::SecureDeletionFailed {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "target has no name"),
        })?;
    let parent_fd = open_directory(parent).map_err(|source| CoreError::SecureDeletionFailed {
        path: path.to_path_buf(),
        source,
    })?;
    let name = CString::new(name.as_bytes()).map_err(|_| CoreError::SecureDeletionFailed {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "target name contains NUL"),
    })?;
    let target_fd = open_directory_at(parent_fd, &name).map_err(|source| {
        close_fd(parent_fd);
        CoreError::SecureDeletionFailed {
            path: path.to_path_buf(),
            source,
        }
    })?;

    let actual = file_identity_from_fd(target_fd).ok_or_else(|| {
        close_fd(target_fd);
        close_fd(parent_fd);
        CoreError::TargetIdentityUnavailable {
            path: path.to_path_buf(),
        }
    })?;
    if actual != expected {
        close_fd(target_fd);
        close_fd(parent_fd);
        return Err(CoreError::TargetIdentityChanged {
            path: path.to_path_buf(),
        });
    }

    let result = remove_tree_at(target_fd);
    close_fd(target_fd);
    if let Err(source) = result {
        close_fd(parent_fd);
        return Err(CoreError::SecureDeletionFailed {
            path: path.to_path_buf(),
            source,
        });
    }

    // The tree is removed through the pinned directory fd. Re-check the name
    // before unlinking it so a replacement is rejected rather than removed.
    let current = identity_at(parent_fd, &name).ok_or_else(|| {
        close_fd(parent_fd);
        CoreError::TargetIdentityUnavailable {
            path: path.to_path_buf(),
        }
    })?;
    if current != expected {
        close_fd(parent_fd);
        return Err(CoreError::TargetIdentityChanged {
            path: path.to_path_buf(),
        });
    }
    let status = unsafe { libc::unlinkat(parent_fd, name.as_ptr(), libc::AT_REMOVEDIR) };
    let result = if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    };
    close_fd(parent_fd);
    result.map_err(|source| CoreError::SecureDeletionFailed {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(all(unix, not(target_os = "android")))]
fn remove_tree_at(fd: std::os::unix::io::RawFd) -> std::io::Result<()> {
    use std::ffi::{CStr, CString};

    let scan_fd = unsafe { libc::dup(fd) };
    if scan_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let directory = unsafe { libc::fdopendir(scan_fd) };
    if directory.is_null() {
        close_fd(scan_fd);
        return Err(std::io::Error::last_os_error());
    }
    let mut entries = Vec::new();
    loop {
        clear_errno();
        let entry = unsafe { libc::readdir(directory) };
        if entry.is_null() {
            let error = std::io::Error::last_os_error();
            unsafe { libc::closedir(directory) };
            if error.raw_os_error().is_some_and(|code| code != 0) {
                return Err(error);
            }
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() != b"." && name.to_bytes() != b".." {
            entries.push(name.to_owned());
        }
    }
    for name in entries {
        let child = CString::new(name.as_bytes()).expect("directory entry cannot contain NUL");
        match open_directory_at(fd, &child) {
            Ok(child_fd) => {
                let result = remove_tree_at(child_fd);
                close_fd(child_fd);
                result?;
                if unsafe { libc::unlinkat(fd, child.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => {
                if unsafe { libc::unlinkat(fd, child.as_ptr(), 0) } != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "android")))]
fn file_identity_from_fd(fd: std::os::unix::io::RawFd) -> Option<FileIdentity> {
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe { libc::fstat(fd, &mut metadata) } != 0 {
        return None;
    }
    let mut file_id = [0; 16];
    file_id[..8].copy_from_slice(&(metadata.st_ino as u64).to_le_bytes());
    Some(FileIdentity {
        volume_serial: metadata.st_dev as u64,
        file_id,
    })
}

#[cfg(all(unix, not(target_os = "android")))]
fn identity_at(parent: std::os::unix::io::RawFd, name: &std::ffi::CStr) -> Option<FileIdentity> {
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe {
        libc::fstatat(
            parent,
            name.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return None;
    }
    if (metadata.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        return None;
    }
    let mut file_id = [0; 16];
    file_id[..8].copy_from_slice(&(metadata.st_ino as u64).to_le_bytes());
    Some(FileIdentity {
        volume_serial: metadata.st_dev as u64,
        file_id,
    })
}

#[cfg(all(unix, not(target_os = "android")))]
fn open_directory(path: &Path) -> std::io::Result<std::os::unix::io::RawFd> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

#[cfg(all(unix, not(target_os = "android")))]
fn open_directory_at(
    parent: std::os::unix::io::RawFd,
    name: &std::ffi::CStr,
) -> std::io::Result<std::os::unix::io::RawFd> {
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

#[cfg(all(unix, not(target_os = "android")))]
fn close_fd(fd: std::os::unix::io::RawFd) {
    unsafe {
        libc::close(fd);
    }
}

#[cfg(all(
    unix,
    not(target_os = "android"),
    any(target_os = "macos", target_os = "ios")
))]
fn clear_errno() {
    unsafe {
        *libc::__error() = 0;
    }
}

#[cfg(all(
    unix,
    not(target_os = "android"),
    not(any(target_os = "macos", target_os = "ios"))
))]
fn clear_errno() {
    unsafe {
        *libc::__errno_location() = 0;
    }
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

    #[cfg(all(unix, not(target_os = "android")))]
    {
        let _ = root;
        let identity = target
            .identity
            .ok_or_else(|| CoreError::TargetIdentityUnavailable {
                path: target.path.clone(),
            })?;
        Ok(SecureTarget {
            backend: BackendTarget::Unix {
                path: target.path.clone(),
                identity,
            },
        })
    }

    #[cfg(not(any(windows, all(unix, not(target_os = "android")))))]
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

        assert_eq!(
            capabilities.secure_identity,
            cfg!(any(windows, all(unix, not(target_os = "android"))))
        );
        assert_eq!(capabilities.secure_identity, capabilities.permanent);
    }

    #[cfg(all(unix, not(target_os = "android")))]
    #[test]
    fn unix_identity_mismatch_fails_closed_before_mutation() {
        let root = std::env::temp_dir().join(format!(
            "uncached-core-unix-identity-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = root.join("target");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("content.txt"), b"content").unwrap();

        let wrong_identity = FileIdentity {
            volume_serial: 0,
            file_id: [0; 16],
        };
        let permanent = SecureTarget {
            backend: BackendTarget::Unix {
                path: path.clone(),
                identity: wrong_identity,
            },
        };
        assert!(matches!(
            permanent.delete_permanently(),
            Err(CoreError::TargetIdentityChanged { .. })
        ));
        assert!(path.is_dir());

        let trash = SecureTarget {
            backend: BackendTarget::Unix {
                path: path.clone(),
                identity: wrong_identity,
            },
        };
        assert!(matches!(
            trash.move_to_trash(),
            Err(CoreError::TargetIdentityChanged { .. })
        ));
        assert!(path.is_dir());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(not(any(windows, all(unix, not(target_os = "android")))))]
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
