use crate::error::{CoreError, CoreResult};
use crate::model::{FileIdentity, ScanTarget};
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

const OPEN_FLAGS: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;

#[derive(Debug)]
pub(super) struct UnixTarget {
    _workspace: OwnedFd,
    _parent: OwnedFd,
    _target: OwnedFd,
    _basename: CString,
}

impl UnixTarget {
    pub(super) fn capabilities(&self) -> super::CleanupCapabilities {
        super::CleanupCapabilities {
            permanent: false,
            trash: false,
        }
    }

    pub(super) fn delete_permanently(self) -> CoreResult<()> {
        Err(CoreError::SecureOperationUnsupported {
            operation: "Unix top-level directory deletion by retained file descriptor".to_owned(),
        })
    }

    pub(super) fn move_to_trash(self) -> CoreResult<()> {
        Err(CoreError::SecureTrashUnavailable)
    }
}

pub(super) fn open_secure_target(_root: &Path, target: &ScanTarget) -> CoreResult<UnixTarget> {
    let root = _root;
    let relative =
        target
            .path
            .strip_prefix(root)
            .map_err(|_| CoreError::SecureTargetOpenFailed {
                path: target.path.clone(),
                source: invalid_input("target is outside workspace"),
            })?;
    let mut components = relative.components().peekable();
    let basename = match components.next_back() {
        Some(Component::Normal(name)) => {
            CString::new(name.as_bytes()).map_err(|_| CoreError::SecureTargetOpenFailed {
                path: target.path.clone(),
                source: invalid_input("target basename contains NUL"),
            })?
        }
        _ => {
            return Err(CoreError::SecureTargetOpenFailed {
                path: target.path.clone(),
                source: invalid_input("target must have a normal basename"),
            })
        }
    };

    let workspace = open_directory_at(libc::AT_FDCWD, root.as_os_str().as_bytes(), &target.path)?;
    let mut parent = duplicate_fd(workspace.as_raw_fd(), &target.path)?;
    for component in components {
        let Component::Normal(name) = component else {
            return Err(CoreError::SecureTargetOpenFailed {
                path: target.path.clone(),
                source: invalid_input("target path contains unsupported component"),
            });
        };
        let name =
            CString::new(name.as_bytes()).map_err(|_| CoreError::SecureTargetOpenFailed {
                path: target.path.clone(),
                source: invalid_input("target component contains NUL"),
            })?;
        let next = open_directory_at(parent.as_raw_fd(), name.as_bytes(), &target.path)?;
        parent = next;
    }

    let target_fd = open_directory_at(parent.as_raw_fd(), basename.as_bytes(), &target.path)?;
    let actual = file_identity_from_fd(target_fd.as_raw_fd(), &target.path)?;
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

    Ok(UnixTarget {
        _workspace: workspace,
        _parent: parent,
        _target: target_fd,
        _basename: basename,
    })
}

fn open_directory_at(parent: RawFd, path: &[u8], display: &Path) -> CoreResult<OwnedFd> {
    let path = CString::new(path).map_err(|_| CoreError::SecureTargetOpenFailed {
        path: display.to_path_buf(),
        source: invalid_input("path contains NUL"),
    })?;
    let fd = unsafe { libc::openat(parent, path.as_ptr(), OPEN_FLAGS | libc::O_NOFOLLOW) };
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
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn duplicate_fd(fd: RawFd, display: &Path) -> CoreResult<OwnedFd> {
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(CoreError::SecureTargetOpenFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

fn file_identity_from_fd(fd: RawFd, path: &Path) -> CoreResult<FileIdentity> {
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe { libc::fstat(fd, &mut metadata) } != 0 {
        return Err(CoreError::SecureTargetOpenFailed {
            path: path.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(FileIdentity {
        volume_serial: metadata.st_dev as u64,
        file_id: (metadata.st_ino as u64)
            .to_le_bytes()
            .into_iter()
            .chain([0; 8])
            .collect::<Vec<_>>()
            .try_into()
            .expect("identity is 16 bytes"),
    })
}

fn invalid_input(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

#[allow(dead_code)]
fn stat_entry_at(parent: RawFd, name: &CString, display: &Path) -> CoreResult<libc::stat> {
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
        return Err(CoreError::SecureEnumerationFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(metadata)
}

#[allow(dead_code)]
fn unlink_entry_at(
    parent: RawFd,
    name: &CString,
    directory: bool,
    display: &Path,
) -> CoreResult<()> {
    let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
    if unsafe { libc::unlinkat(parent, name.as_ptr(), flags) } != 0 {
        return Err(CoreError::SecureDeletionFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(())
}
