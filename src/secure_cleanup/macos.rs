use crate::error::{CoreError, CoreResult};
use crate::model::{FileIdentity, ScanTarget};
use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

const OPEN_FLAGS: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;

#[derive(Debug)]
pub(super) struct MacosTarget {
    _workspace: OwnedFd,
    _parent: OwnedFd,
    _target: OwnedFd,
    _basename: CString,
}

impl MacosTarget {
    pub(super) fn capabilities(&self) -> super::CleanupCapabilities {
        super::CleanupCapabilities {
            permanent: false,
            trash: false,
        }
    }

    pub(super) fn delete_permanently(self) -> CoreResult<()> {
        Err(CoreError::SecureOperationUnsupported {
            operation: "macOS top-level directory deletion by retained file descriptor".to_owned(),
        })
    }

    pub(super) fn move_to_trash(self) -> CoreResult<()> {
        Err(CoreError::SecureTrashUnavailable)
    }
}

pub(super) fn open_secure_target(root: &Path, target: &ScanTarget) -> CoreResult<MacosTarget> {
    let relative = target
        .path
        .strip_prefix(root)
        .map_err(|_| open_error(target, "target is outside workspace"))?;
    let mut components = relative.components().peekable();
    let basename = match components.next_back() {
        Some(Component::Normal(name)) => CString::new(name.as_bytes())
            .map_err(|_| open_error(target, "target basename contains NUL"))?,
        _ => return Err(open_error(target, "target must have a normal basename")),
    };

    let workspace = open_workspace(root, &target.path)?;
    let mut parent = duplicate_fd(workspace.as_raw_fd(), &target.path)?;
    for component in components {
        let Component::Normal(name) = component else {
            return Err(open_error(
                target,
                "target path contains unsupported component",
            ));
        };
        parent = open_directory_at(parent.as_raw_fd(), name.as_bytes(), &target.path)?;
    }

    let target_fd = open_directory_at(parent.as_raw_fd(), basename.as_bytes(), &target.path)?;
    let actual = identity_from_fd(target_fd.as_raw_fd(), &target.path)?;
    let expected = target
        .identity
        .ok_or_else(|| CoreError::TargetIdentityUnavailable {
            path: target.path.clone(),
        })?;
    if actual != expected {
        return Err(CoreError::TargetIdentityChanged {
            path: target.path.clone(),
        });
    }
    validate_children(target_fd.as_raw_fd(), &target.path)?;

    Ok(MacosTarget {
        _workspace: workspace,
        _parent: parent,
        _target: target_fd,
        _basename: basename,
    })
}

fn open_error(target: &ScanTarget, message: &str) -> CoreError {
    CoreError::SecureTargetOpenFailed {
        path: target.path.clone(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, message),
    }
}

fn open_workspace(root: &Path, display: &Path) -> CoreResult<OwnedFd> {
    let mut current = if root.is_absolute() {
        open_directory_at(libc::AT_FDCWD, b"/", display)?
    } else {
        open_directory_at(libc::AT_FDCWD, b".", display)?
    };
    for component in root.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                current = open_directory_at(current.as_raw_fd(), name.as_bytes(), display)?;
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(CoreError::SecureTargetOpenFailed {
                    path: display.to_path_buf(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "workspace contains unsupported component",
                    ),
                });
            }
        }
    }
    Ok(current)
}

fn open_directory_at(parent: RawFd, bytes: &[u8], display: &Path) -> CoreResult<OwnedFd> {
    let name = CString::new(bytes).map_err(|_| CoreError::SecureTargetOpenFailed {
        path: display.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"),
    })?;
    // SAFETY: parent is owned open directory FD and name is NUL-terminated.
    let fd = unsafe { libc::openat(parent, name.as_ptr(), OPEN_FLAGS | libc::O_NOFOLLOW) };
    if fd < 0 {
        let source = std::io::Error::last_os_error();
        if source.raw_os_error() == Some(libc::ELOOP) {
            return Err(CoreError::TargetReparsePoint {
                path: display.to_path_buf(),
            });
        }
        return Err(CoreError::SecureTargetOpenFailed {
            path: display.to_path_buf(),
            source,
        });
    }
    // SAFETY: successful openat returned a unique owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn duplicate_fd(fd: RawFd, display: &Path) -> CoreResult<OwnedFd> {
    // SAFETY: fd is valid for this call; fcntl creates independent ownership.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(CoreError::SecureTargetOpenFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: successful fcntl returned a new owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

fn identity_from_fd(fd: RawFd, path: &Path) -> CoreResult<FileIdentity> {
    // SAFETY: zeroed stat is initialized storage for fstat.
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    // SAFETY: fd is valid and stat points to writable storage.
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return Err(CoreError::SecureTargetOpenFailed {
            path: path.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(FileIdentity {
        volume_serial: stat.st_dev as u64,
        file_id: (stat.st_ino as u64)
            .to_le_bytes()
            .into_iter()
            .chain([0; 8])
            .collect::<Vec<_>>()
            .try_into()
            .expect("identity is 16 bytes"),
    })
}

fn validate_children(parent: RawFd, display: &Path) -> CoreResult<()> {
    let duplicate = duplicate_fd(parent, display)?;
    let raw = duplicate.into_raw_fd();
    // SAFETY: raw descriptor ownership transfers to fdopendir.
    let stream = unsafe { libc::fdopendir(raw) };
    if stream.is_null() {
        // SAFETY: fdopendir failed and retained descriptor ownership.
        unsafe { libc::close(raw) };
        return Err(enumeration_error(display));
    }
    loop {
        // SAFETY: __error returns macOS thread-local errno storage.
        unsafe { *libc::__error() = 0 };
        // SAFETY: stream remains valid until closed below.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            // SAFETY: __error returns macOS thread-local errno storage.
            let error = unsafe { *libc::__error() };
            // SAFETY: stream is closed exactly once.
            unsafe { libc::closedir(stream) };
            return if error == 0 {
                Ok(())
            } else {
                Err(CoreError::SecureEnumerationFailed {
                    path: display.to_path_buf(),
                    source: std::io::Error::from_raw_os_error(error),
                })
            };
        }
        // SAFETY: d_name is NUL-terminated and valid until next readdir call.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        let child_display = display.join(name.to_string_lossy().as_ref());
        match open_directory_at(parent, name.to_bytes(), &child_display) {
            Ok(child) => {
                if let Err(error) = validate_children(child.as_raw_fd(), &child_display) {
                    // SAFETY: stream remains valid and is closed exactly once.
                    unsafe { libc::closedir(stream) };
                    return Err(error);
                }
            }
            Err(CoreError::SecureTargetOpenFailed { source, .. })
                if source.raw_os_error() == Some(libc::ENOTDIR) => {}
            Err(error) => {
                // SAFETY: stream remains valid and is closed exactly once.
                unsafe { libc::closedir(stream) };
                return Err(error);
            }
        }
    }
}

fn enumeration_error(path: &Path) -> CoreError {
    CoreError::SecureEnumerationFailed {
        path: path.to_path_buf(),
        source: std::io::Error::last_os_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::open_secure_target;
    use crate::error::CoreError;
    use crate::filesystem::file_identity;
    use crate::model::{Category, ScanTarget};
    use std::fs;

    #[test]
    fn macos_secure_open_rejects_identity_mismatch() {
        let root = fixture("identity");
        let path = root.join("target");
        fs::create_dir(&path).unwrap();
        let target = ScanTarget::new(path, Category::NodeModules, 0).with_identity(Some(
            crate::model::FileIdentity {
                volume_serial: 0,
                file_id: [0; 16],
            },
        ));
        assert!(matches!(
            open_secure_target(&root, &target),
            Err(CoreError::TargetIdentityChanged { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn macos_secure_open_rejects_symlink_target() {
        let root = fixture("symlink");
        fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink("real", root.join("target")).unwrap();
        let target = ScanTarget::new(root.join("target"), Category::NodeModules, 0)
            .with_identity(file_identity(&root.join("real")));
        assert!(matches!(
            open_secure_target(&root, &target),
            Err(CoreError::TargetReparsePoint { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn macos_secure_operations_are_unsupported_without_mutation() {
        let root = fixture("unsupported");
        let path = root.join("target");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("child"), b"data").unwrap();
        let target = ScanTarget::new(path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&path));
        let secure = open_secure_target(&root, &target).unwrap();
        assert!(!secure.capabilities().permanent);
        assert!(!secure.capabilities().trash);
        assert!(matches!(
            secure.delete_permanently(),
            Err(CoreError::SecureOperationUnsupported { .. })
        ));
        assert!(path.join("child").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn macos_trash_capability_is_unavailable_without_mutation() {
        let root = fixture("unsupported-trash");
        let path = root.join("target");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("child"), b"data").unwrap();
        let target = ScanTarget::new(path.clone(), Category::NodeModules, 0)
            .with_identity(Some(file_identity(&path)));
        let secure = open_secure_target(&root, &target).unwrap();

        assert!(!secure.capabilities().trash);
        assert!(matches!(
            secure.move_to_trash(),
            Err(CoreError::SecureTrashUnavailable)
        ));
        assert!(path.is_dir());
        assert_eq!(fs::read(path.join("child")).unwrap(), b"data");
        let _ = fs::remove_dir_all(root);
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("uncached-core-macos-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        root
    }
}
