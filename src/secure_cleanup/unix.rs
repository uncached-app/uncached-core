use crate::error::{CoreError, CoreResult};
use crate::model::{FileIdentity, ScanTarget};
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
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

pub(super) fn open_secure_target(root: &Path, target: &ScanTarget) -> CoreResult<UnixTarget> {
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

    let workspace = open_workspace(root, &target.path)?;
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

    validate_children(target_fd.as_raw_fd(), &target.path)?;

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
    // SAFETY: `parent` is an open directory FD owned by this module, and `path`
    // is a NUL-terminated CString. O_NOFOLLOW prevents symlink traversal.
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
    // SAFETY: successful openat returned a unique owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Opens every workspace component without following symlinked ancestors.
fn open_workspace(root: &Path, display: &Path) -> CoreResult<OwnedFd> {
    let absolute = root.is_absolute();
    let mut components = root.components();
    let mut current = if absolute {
        open_directory_at(libc::AT_FDCWD, b"/", display)?
    } else {
        open_directory_at(libc::AT_FDCWD, b".", display)?
    };

    for component in components.by_ref() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let name = CString::new(name.as_bytes()).map_err(|_| {
                    CoreError::SecureTargetOpenFailed {
                        path: display.to_path_buf(),
                        source: invalid_input("workspace component contains NUL"),
                    }
                })?;
                current = open_directory_at(current.as_raw_fd(), name.as_bytes(), display)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(CoreError::SecureTargetOpenFailed {
                    path: display.to_path_buf(),
                    source: invalid_input("workspace contains unsupported component"),
                });
            }
        }
    }
    Ok(current)
}

/// Duplicates a valid open descriptor and transfers ownership to `OwnedFd`.
fn duplicate_fd(fd: RawFd, display: &Path) -> CoreResult<OwnedFd> {
    // SAFETY: `fd` is required to be a valid open descriptor for the duration
    // of this call; fcntl creates a new descriptor with independent ownership.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(CoreError::SecureTargetOpenFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: successful fcntl returned a new descriptor owned by this call.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

/// Reads identity from a valid open descriptor into initialized stat storage.
fn file_identity_from_fd(fd: RawFd, path: &Path) -> CoreResult<FileIdentity> {
    // SAFETY: zeroed libc::stat is valid initialized storage for fstat.
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    // SAFETY: `fd` is a valid descriptor and `metadata` points to writable storage.
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

/// Enumerates children through a directory FD; returned names contain no NUL.
fn enumerate_children(parent: RawFd, display: &Path) -> CoreResult<Vec<CString>> {
    let duplicate = duplicate_fd(parent, display)?;
    let directory = duplicate.into_raw_fd();
    // SAFETY: `directory` is a valid descriptor transferred to fdopendir.
    let stream = unsafe { libc::fdopendir(directory) };
    if stream.is_null() {
        // SAFETY: fdopendir failed and retains ownership of descriptor.
        unsafe { libc::close(directory) };
        return Err(CoreError::SecureEnumerationFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }

    let mut names = Vec::new();
    loop {
        set_errno(0);
        // SAFETY: `stream` is a live DIR* owned until closed below.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            let errno = errno();
            if errno != 0 {
                // SAFETY: stream remains live and is closed exactly once here.
                unsafe { libc::closedir(stream) };
                return Err(CoreError::SecureEnumerationFailed {
                    path: display.to_path_buf(),
                    source: std::io::Error::from_raw_os_error(errno),
                });
            }
            break;
        }
        // SAFETY: d_name is NUL-terminated by readdir and valid until next call.
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() != b"." && name.to_bytes() != b".." {
            names.push(name.to_owned());
        }
    }
    // SAFETY: stream remains live and is closed exactly once here.
    if unsafe { libc::closedir(stream) } != 0 {
        return Err(CoreError::SecureEnumerationFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(names)
}

/// Recursively validates children relative to `parent`, rejecting symlink entries.
fn validate_children(parent: RawFd, display: &Path) -> CoreResult<()> {
    for name in enumerate_children(parent, display)? {
        let child_display = display.join(name.to_string_lossy().as_ref());
        let child = match open_directory_at(parent, name.as_bytes(), &child_display) {
            Ok(child) => Some(child),
            Err(CoreError::SecureTargetOpenFailed { source, .. })
                if source.raw_os_error() == Some(libc::ENOTDIR) =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        if let Some(child) = child {
            validate_children(child.as_raw_fd(), &child_display)?;
        }
    }
    Ok(())
}

fn set_errno(value: libc::c_int) {
    // SAFETY: libc exposes thread-local errno storage for these targets.
    unsafe { *libc::__errno_location() = value };
}

fn errno() -> libc::c_int {
    // SAFETY: libc exposes thread-local errno storage for these targets.
    unsafe { *libc::__errno_location() }
}

#[cfg(test)]
mod tests {
    use super::{open_directory_at, open_secure_target, validate_children};
    use crate::error::CoreError;
    use crate::filesystem::file_identity;
    use crate::model::{Category, ScanTarget};
    use std::fs;
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;

    #[test]
    fn rejects_target_when_identity_does_not_match() {
        let fixture = Fixture::new("identity-mismatch");
        let target = fixture
            .target("target")
            .with_identity(Some(crate::model::FileIdentity {
                volume_serial: 0,
                file_id: [0; 16],
            }));

        assert!(matches!(
            open_secure_target(&fixture.root, &target),
            Err(CoreError::TargetIdentityChanged { .. })
        ));
    }

    #[test]
    fn rejects_direct_symlink_target() {
        let fixture = Fixture::new("direct-symlink");
        fs::create_dir(fixture.root.join("real")).expect("real target should exist");
        #[cfg(unix)]
        std::os::unix::fs::symlink("real", fixture.root.join("target"))
            .expect("target symlink should be created");
        let identity = file_identity(&fixture.root.join("real"));
        let target = ScanTarget::new(fixture.root.join("target"), Category::NodeModules, 0)
            .with_identity(identity);

        assert!(matches!(
            open_secure_target(&fixture.root, &target),
            Err(CoreError::TargetReparsePoint { .. })
        ));
    }

    #[test]
    fn rejects_symlinked_ancestor() {
        let fixture = Fixture::new("ancestor-symlink");
        let real = fixture.root.join("real");
        fs::create_dir(&real).expect("real ancestor should exist");
        fs::create_dir(real.join("target")).expect("target should exist");
        std::os::unix::fs::symlink("real", fixture.root.join("link"))
            .expect("ancestor symlink should be created");
        let target = fixture.root.join("link").join("target");
        let scan_target = ScanTarget::new(target, Category::NodeModules, 0)
            .with_identity(file_identity(&real.join("target")));

        assert!(matches!(
            open_secure_target(&fixture.root, &scan_target),
            Err(CoreError::TargetReparsePoint { .. })
        ));
    }

    #[test]
    fn unsupported_top_level_deletion_does_not_mutate_target() {
        let fixture = Fixture::new("unsupported-delete");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).expect("target should exist");
        fs::write(target_path.join("child"), b"data").expect("child should exist");
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&target_path));
        let secure_target =
            open_secure_target(&fixture.root, &target).expect("identity-bound target should open");
        assert_eq!(
            secure_target.capabilities(),
            super::super::CleanupCapabilities {
                permanent: false,
                trash: false,
            }
        );

        assert!(matches!(
            secure_target.delete_permanently(),
            Err(CoreError::SecureOperationUnsupported { .. })
        ));
        assert!(target_path.join("child").is_file());
    }

    #[test]
    fn trash_capability_is_unavailable_without_mutation() {
        let fixture = Fixture::new("unsupported-trash");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).expect("target should exist");
        fs::write(target_path.join("child"), b"data").expect("child should exist");
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(Some(file_identity(&target_path)));
        let secure_target =
            open_secure_target(&fixture.root, &target).expect("identity-bound target should open");

        assert!(!secure_target.capabilities().trash);
        assert!(matches!(
            secure_target.move_to_trash(),
            Err(CoreError::SecureTrashUnavailable)
        ));
        assert!(target_path.is_dir());
        assert_eq!(fs::read(target_path.join("child")).unwrap(), b"data");
    }

    #[test]
    fn validates_nested_children_relative_to_verified_directory_fd() {
        let fixture = Fixture::new("recursive-validation");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).expect("target should exist");
        fs::create_dir(target_path.join("nested")).expect("nested directory should exist");
        fs::write(target_path.join("nested").join("child"), b"data").expect("child should exist");
        let target = fixture
            .target("other")
            .with_identity(file_identity(&fixture.root.join("other")));
        let secure_target =
            open_secure_target(&fixture.root, &target).expect("identity-bound target should open");
        let target_fd =
            open_directory_at(secure_target._parent.as_raw_fd(), b"target", &target_path)
                .expect("nested target should open");

        validate_children(target_fd.as_raw_fd(), &target_path)
            .expect("nested children should validate");
        assert!(fs::read_dir(&target_path)
            .expect("target should remain readable")
            .next()
            .is_some());
    }

    #[test]
    fn secure_open_rejects_symlink_child_during_validation() {
        let fixture = Fixture::new("child-symlink");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).expect("target should exist");
        fs::create_dir(fixture.root.join("real")).expect("real directory should exist");
        std::os::unix::fs::symlink("../real", target_path.join("link"))
            .expect("child symlink should be created");
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&target_path));

        assert!(matches!(
            open_secure_target(&fixture.root, &target),
            Err(CoreError::TargetReparsePoint { .. })
        ));
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "uncached-core-secure-{label}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir(&root).expect("fixture root should be created");
            Self { root }
        }

        fn target(&self, name: &str) -> ScanTarget {
            let path = self.root.join(name);
            fs::create_dir(&path).expect("target should be created");
            ScanTarget::new(path, Category::NodeModules, 0)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
