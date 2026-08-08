use crate::error::{CoreError, CoreResult};
use crate::model::{FileIdentity, ScanTarget};
use std::ffi::c_void;
use std::ffi::OsString;
use std::mem::{size_of, MaybeUninit};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, FileIdExtdDirectoryInfo, GetFileInformationByHandleEx,
    SetFileInformationByHandle, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_TAG_INFO, FILE_DISPOSITION_FLAG_DELETE,
    FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    FILE_DISPOSITION_INFO_EX, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_FLAG_POSIX_SEMANTICS, FILE_ID_EXTD_DIR_INFO, FILE_ID_INFO, FILE_LIST_DIRECTORY,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

#[derive(Debug)]
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle owns a successful CreateFileW result.
        unsafe { CloseHandle(self.0) };
    }
}

#[derive(Debug)]
struct Entry {
    handle: Handle,
    is_directory: bool,
    identity: FileIdentity,
    path: PathBuf,
}

#[derive(Debug)]
pub(super) struct WindowsTarget {
    _ancestors: Vec<Handle>,
    entries: Vec<Entry>,
}

impl WindowsTarget {
    pub(super) fn capabilities(&self) -> super::CleanupCapabilities {
        super::CleanupCapabilities {
            permanent: true,
            trash: false,
        }
    }

    pub(super) fn delete_permanently(mut self) -> CoreResult<()> {
        for entry in self.entries.drain(..).rev() {
            let info = FILE_DISPOSITION_INFO_EX {
                Flags: FILE_DISPOSITION_FLAG_DELETE
                    | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
                    | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
            };
            // SAFETY: entry.handle is valid, and info is initialized for this class.
            let ok = unsafe {
                SetFileInformationByHandle(
                    entry.handle.0,
                    windows_sys::Win32::Storage::FileSystem::FileDispositionInfoEx,
                    &info as *const _ as *const c_void,
                    size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
                )
            };
            if ok == 0 {
                return Err(CoreError::SecureDeletionFailed {
                    path: entry.path,
                    source: std::io::Error::last_os_error(),
                });
            }
        }
        Ok(())
    }

    pub(super) fn move_to_trash(self) -> CoreResult<()> {
        Err(CoreError::SecureTrashUnavailable)
    }
}

pub(super) fn open_secure_target(root: &Path, target: &ScanTarget) -> CoreResult<WindowsTarget> {
    let expected = target
        .identity
        .ok_or_else(|| CoreError::TargetIdentityUnavailable {
            path: target.path.clone(),
        })?;
    let relative = target
        .path
        .strip_prefix(root)
        .map_err(|_| open_error(target, "target is outside workspace"))?;
    let components: Vec<_> = relative.components().collect();
    let Some(Component::Normal(basename)) = components.last() else {
        return Err(open_error(target, "target must have a normal basename"));
    };

    let mut ancestors = Vec::new();
    let mut current = root.to_path_buf();
    let root_entry = open_directory(root, &target.path)?;
    ancestors.push(root_entry.handle);
    for component in &components[..components.len() - 1] {
        let Component::Normal(name) = component else {
            return Err(open_error(
                target,
                "target path contains unsupported component",
            ));
        };
        current.push(name);
        ancestors.push(open_directory(&current, &target.path)?.handle);
    }

    let target_path = current.join(basename);
    let target_entry = open_entry(&target_path, &target.path)?;
    if !target_entry.is_directory {
        return Err(open_error(
            target,
            "secure cleanup target is not a directory",
        ));
    }
    if identity_from_handle(target_entry.handle.0, &target.path)? != expected {
        return Err(CoreError::TargetIdentityChanged {
            path: target.path.clone(),
        });
    }

    let mut entries = Vec::new();
    collect_entries(target_entry, &mut entries)?;
    Ok(WindowsTarget {
        _ancestors: ancestors,
        entries,
    })
}

fn collect_entries(entry: Entry, entries: &mut Vec<Entry>) -> CoreResult<()> {
    if entry.is_directory {
        let children =
            enumerate_directory(entry.handle.0, &entry.path, entry.identity.volume_serial)?;
        entries.push(entry);
        for child in children {
            collect_entries(child, entries)?;
        }
    } else {
        entries.push(entry);
    }
    Ok(())
}

fn open_directory(path: &Path, display: &Path) -> CoreResult<Entry> {
    let entry = open_entry(path, display)?;
    if !entry.is_directory {
        return Err(open_error_path(
            display,
            "path component is not a directory",
        ));
    }
    Ok(entry)
}

fn open_entry(path: &Path, display: &Path) -> CoreResult<Entry> {
    let wide = wide_path(path);
    let open = |access| {
        // SAFETY: wide is NUL-terminated; all other arguments are fixed valid flags.
        unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_POSIX_SEMANTICS,
                std::ptr::null_mut(),
            )
        }
    };
    // FILE_LIST_DIRECTORY is required for directory enumeration but is not a
    // valid desired access for every regular-file provider.
    let mut handle = open(DELETE | FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY);
    if handle == INVALID_HANDLE_VALUE {
        handle = open(DELETE | FILE_READ_ATTRIBUTES);
    }
    if handle == INVALID_HANDLE_VALUE {
        return Err(CoreError::SecureTargetOpenFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    let handle = Handle(handle);
    let attributes = attributes_from_handle(handle.0, display)?;
    if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || attributes.ReparseTag != 0 {
        return Err(CoreError::TargetReparsePoint {
            path: display.to_path_buf(),
        });
    }
    let identity = identity_from_handle(handle.0, display)?;
    Ok(Entry {
        is_directory: attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        identity,
        handle,
        path: path.to_path_buf(),
    })
}

fn attributes_from_handle(handle: HANDLE, path: &Path) -> CoreResult<FILE_ATTRIBUTE_TAG_INFO> {
    let mut info = MaybeUninit::<FILE_ATTRIBUTE_TAG_INFO>::zeroed();
    // SAFETY: buffer is valid writable storage of the requested class size.
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            info.as_mut_ptr() as *mut c_void,
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(CoreError::SecureHandleQueryFailed {
            path: path.to_path_buf(),
            operation: "attributes",
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: successful API call initialized buffer.
    Ok(unsafe { info.assume_init() })
}

fn identity_from_handle(handle: HANDLE, path: &Path) -> CoreResult<FileIdentity> {
    let mut info = MaybeUninit::<FILE_ID_INFO>::zeroed();
    // SAFETY: buffer is valid writable storage of the requested class size.
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            windows_sys::Win32::Storage::FileSystem::FileIdInfo,
            info.as_mut_ptr() as *mut c_void,
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(CoreError::SecureHandleQueryFailed {
            path: path.to_path_buf(),
            operation: "identity",
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: successful API call initialized buffer.
    let info = unsafe { info.assume_init() };
    Ok(FileIdentity {
        volume_serial: info.VolumeSerialNumber,
        file_id: info.FileId.Identifier,
    })
}

fn enumerate_directory(
    handle: HANDLE,
    display: &Path,
    volume_serial: u64,
) -> CoreResult<Vec<Entry>> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut result = Vec::new();
    loop {
        // SAFETY: buffer is writable and handle is an open directory handle.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdExtdDirectoryInfo,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as u32,
            )
        };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(18) {
                break;
            }
            return Err(CoreError::SecureEnumerationFailed {
                path: display.to_path_buf(),
                source: error,
            });
        }
        let mut offset = 0usize;
        loop {
            // SAFETY: Windows returns aligned entries whose offset stays within buffer.
            let info = unsafe { &*(buffer.as_ptr().add(offset) as *const FILE_ID_EXTD_DIR_INFO) };
            let name_len = info.FileNameLength as usize / 2;
            let name = unsafe { std::slice::from_raw_parts(info.FileName.as_ptr(), name_len) };
            if name != [46] && name != [46, 46] {
                let name = os_string_from_wide(name);
                let path = display.join(&name);
                let entry = open_entry(&path, &path)?;
                let enumerated = FileIdentity {
                    volume_serial,
                    file_id: info.FileId.Identifier,
                };
                if !identity_matches(enumerated, entry.identity) {
                    return Err(CoreError::TargetIdentityChanged { path });
                }
                result.push(entry);
            }
            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
            if offset >= buffer.len() {
                return Err(CoreError::SecureEnumerationFailed {
                    path: display.to_path_buf(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid directory information offset",
                    ),
                });
            }
        }
    }
    Ok(result)
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn os_string_from_wide(name: &[u16]) -> OsString {
    OsString::from_wide(name)
}

fn identity_matches(enumerated: FileIdentity, opened: FileIdentity) -> bool {
    enumerated == opened
}

fn open_error(target: &ScanTarget, message: &str) -> CoreError {
    open_error_path(&target.path, message)
}

fn open_error_path(path: &Path, message: &str) -> CoreError {
    CoreError::SecureTargetOpenFailed {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, message),
    }
}

#[cfg(test)]
mod tests {
    use super::{identity_matches, open_secure_target, os_string_from_wide};
    use crate::error::CoreError;
    use crate::filesystem::file_identity;
    use crate::model::{Category, ScanTarget};
    use std::ffi::OsString;
    use std::fs;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    #[test]
    fn directory_record_name_preserves_unpaired_utf16_units() {
        let name = os_string_from_wide(&[b'f' as u16, 0xd800, b'x' as u16]);
        assert_eq!(
            name,
            OsString::from_wide(&[b'f' as u16, 0xd800, b'x' as u16])
        );
    }

    #[test]
    fn directory_record_identity_must_match_open_handle() {
        let expected = crate::model::FileIdentity {
            volume_serial: 7,
            file_id: [3; 16],
        };
        assert!(identity_matches(expected, expected));
        assert!(!identity_matches(
            expected,
            crate::model::FileIdentity {
                volume_serial: 7,
                file_id: [4; 16],
            }
        ));
    }

    #[test]
    fn secure_open_retains_stable_identity_and_enables_permanent_cleanup() {
        let fixture = Fixture::new("stable");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("child.txt"), b"data").unwrap();
        let target = ScanTarget::new(target_path, Category::NodeModules, 0)
            .with_identity(file_identity(&fixture.root.join("target")));
        let secure = open_secure_target(&fixture.root, &target).unwrap();
        assert!(secure.capabilities().permanent);
        assert!(!secure.capabilities().trash);
    }

    #[test]
    fn secure_open_rejects_identity_mismatch_and_replacement() {
        let fixture = Fixture::new("identity");
        let original = fixture.root.join("target");
        fs::create_dir(&original).unwrap();
        let target = ScanTarget::new(original.clone(), Category::NodeModules, 0).with_identity(
            Some(crate::model::FileIdentity {
                volume_serial: 0,
                file_id: [0; 16],
            }),
        );
        assert!(matches!(
            open_secure_target(&fixture.root, &target),
            Err(CoreError::TargetIdentityChanged { .. })
        ));
        let identity = file_identity(&original);
        fs::rename(&original, fixture.root.join("old")).unwrap();
        fs::create_dir(&original).unwrap();
        let replaced = ScanTarget::new(original, Category::NodeModules, 0).with_identity(identity);
        assert!(matches!(
            open_secure_target(&fixture.root, &replaced),
            Err(CoreError::TargetIdentityChanged { .. })
        ));
    }

    #[test]
    fn secure_open_rejects_direct_and_ancestor_reparse_points() {
        let fixture = Fixture::new("reparse");
        fs::create_dir(fixture.root.join("real")).unwrap();
        if let Err(error) = std::os::windows::fs::symlink_dir("real", fixture.root.join("direct")) {
            if error.raw_os_error() == Some(1314) {
                return;
            }
            panic!("creating direct reparse fixture failed: {error}");
        }
        let direct = ScanTarget::new(fixture.root.join("direct"), Category::NodeModules, 0)
            .with_identity(file_identity(&fixture.root.join("real")));
        assert!(matches!(
            open_secure_target(&fixture.root, &direct),
            Err(CoreError::TargetReparsePoint { .. })
        ));
        fs::create_dir(fixture.root.join("real").join("target")).unwrap();
        if let Err(error) = std::os::windows::fs::symlink_dir("real", fixture.root.join("ancestor"))
        {
            if error.raw_os_error() == Some(1314) {
                return;
            }
            panic!("creating ancestor reparse fixture failed: {error}");
        }
        let ancestor = ScanTarget::new(
            fixture.root.join("ancestor").join("target"),
            Category::NodeModules,
            0,
        )
        .with_identity(file_identity(&fixture.root.join("real").join("target")));
        assert!(matches!(
            open_secure_target(&fixture.root, &ancestor),
            Err(CoreError::TargetReparsePoint { .. })
        ));
    }

    #[test]
    fn permanent_cleanup_removes_verified_tree_through_retained_handles() {
        let fixture = Fixture::new("handle-delete");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("original.txt"), b"original").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&target_path));
        let secure = open_secure_target(&fixture.root, &target).unwrap();
        secure.delete_permanently().unwrap();
        assert!(!target_path.exists());
    }

    #[test]
    fn unsupported_capability_does_not_mutate_filesystem() {
        let secure = super::super::SecureTarget {
            backend: super::super::BackendTarget::Unsupported,
        };
        assert!(!secure.capabilities().permanent);
        assert!(!secure.capabilities().trash);
        assert!(matches!(
            secure.delete_permanently(),
            Err(CoreError::SecureOperationUnsupported { .. })
        ));
    }

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "uncached-core-windows-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir(&root).unwrap();
            Self { root }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
