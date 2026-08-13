use crate::error::{CoreError, CoreResult};
use crate::model::FileIdentity;
use crate::parallel::worker_pool;
use jwalk::{Parallelism, WalkDir};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub fn canonical_workspace(root: &Path) -> CoreResult<PathBuf> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| CoreError::InvalidWorkspace(root.display().to_string()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CoreError::InvalidWorkspace(root.display().to_string()));
    }
    let canonical = fs::canonicalize(root).map_err(CoreError::from)?;
    #[cfg(target_os = "linux")]
    if is_protected_linux_workspace(&canonical) {
        return Err(CoreError::InvalidWorkspace(canonical.display().to_string()));
    }
    Ok(canonical)
}

#[cfg(target_os = "linux")]
fn is_protected_linux_workspace(path: &Path) -> bool {
    path == Path::new("/")
        || [
            "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/root", "/run", "/sbin", "/sys",
            "/usr", "/var",
        ]
        .iter()
        .map(Path::new)
        .any(|protected| path.starts_with(protected))
}

pub fn is_protected_directory(name: &str) -> bool {
    name.eq_ignore_ascii_case("$RECYCLE.BIN")
        || name.eq_ignore_ascii_case("System Volume Information")
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirectorySizeReport {
    /// Sum of file sizes that were read successfully.
    pub bytes: u64,
    /// Number of entries that could not be read, or size additions that overflowed.
    pub errors: u64,
}

impl DirectorySizeReport {
    /// Returns whether every entry contributed to the reported size.
    pub fn is_complete(&self) -> bool {
        self.errors == 0
    }
}

fn add_file_size(report: &mut DirectorySizeReport, bytes: u64) {
    if let Some(total) = report.bytes.checked_add(bytes) {
        report.bytes = total;
    } else {
        // Keep the partial total bounded, but make the loss visible to callers.
        report.bytes = u64::MAX;
        report.errors = report.errors.saturating_add(1);
    }
}

fn size_parallelism() -> Parallelism {
    match worker_pool() {
        // `busy_timeout: None` keeps a saturated pool from aborting the walk:
        // an aborted walk would report a silently truncated size.
        Some(pool) => Parallelism::RayonExistingPool {
            pool,
            busy_timeout: None,
        },
        None => Parallelism::Serial,
    }
}

pub fn directory_size_report(root: &Path) -> CoreResult<DirectorySizeReport> {
    directory_size_report_with_cancel(root, &AtomicBool::new(false))?.ok_or_else(|| {
        CoreError::Io(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "directory sizing cancelled",
        ))
    })
}

/// Sizes a directory tree and returns `None` when cancellation is observed.
/// Cancellation is checked before each entry is inspected, so callers do not
/// have to wait for the remaining tree to be walked.
pub fn directory_size_report_with_cancel(
    root: &Path,
    cancelled: &AtomicBool,
) -> CoreResult<Option<DirectorySizeReport>> {
    WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(size_parallelism())
        .into_iter()
        .try_fold(DirectorySizeReport::default(), |mut report, entry| {
            if cancelled.load(Ordering::Acquire) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "directory sizing cancelled",
                ));
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    report.errors = report.errors.saturating_add(1);
                    return Ok(report);
                }
            };
            match entry.metadata() {
                Ok(metadata) if metadata.is_file() => {
                    add_file_size(&mut report, metadata.len());
                }
                Ok(_) => {}
                Err(_) => report.errors = report.errors.saturating_add(1),
            }
            Ok(report)
        })
        .map(Some)
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::Interrupted {
                Ok(None)
            } else {
                Err(CoreError::Io(error))
            }
        })
}

pub fn file_identity(path: &Path) -> Option<FileIdentity> {
    #[cfg(unix)]
    {
        let metadata = fs::symlink_metadata(path).ok()?;
        use std::os::unix::fs::MetadataExt;
        let mut file_id = [0; 16];
        file_id[..8].copy_from_slice(&metadata.ino().to_le_bytes());
        Some(FileIdentity {
            volume_serial: metadata.dev(),
            file_id,
        })
    }
    #[cfg(windows)]
    {
        windows_file_identity(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> Option<FileIdentity> {
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Component;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FileAttributeTagInfo, FileIdInfo, GetFileAttributesW,
        GetFileInformationByHandleEx, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_128, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    let mut component_path = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                component_path.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => component_path.push(component.as_os_str()),
            Component::Normal(name) => {
                component_path.push(name);
                let metadata = fs::symlink_metadata(&component_path).ok()?;
                if metadata.file_type().is_symlink() {
                    return None;
                }
                let component_path_wide: Vec<u16> = component_path
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let attributes = unsafe { GetFileAttributesW(component_path_wide.as_ptr()) };
                if attributes == u32::MAX || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return None;
                }
            }
        }
    }

    let path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let _handle = Handle(handle);

    let mut attributes = FILE_ATTRIBUTE_TAG_INFO {
        FileAttributes: 0,
        ReparseTag: 0,
    };
    let attributes_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut attributes as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } != 0;
    if !attributes_ok
        || attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return None;
    }

    let mut information = FILE_ID_INFO {
        VolumeSerialNumber: 0,
        FileId: FILE_ID_128 {
            Identifier: [0; 16],
        },
    };
    let information_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } != 0;
    if !information_ok {
        return None;
    }

    Some(FileIdentity {
        volume_serial: information.VolumeSerialNumber,
        file_id: information.FileId.Identifier,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::{add_file_size, file_identity, DirectorySizeReport};
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    #[test]
    fn file_identity_maps_unix_device_and_inode() {
        let path = unique_test_path();
        fs::write(&path, b"identity").expect("test file should be created");

        let metadata = fs::symlink_metadata(&path).expect("test file metadata should exist");
        let identity = file_identity(&path).expect("test file should have identity");

        assert_eq!(identity.volume_serial, metadata.dev());
        assert_eq!(identity.file_id[..8], metadata.ino().to_le_bytes());
        assert_eq!(identity.file_id[8..], [0; 8]);

        fs::remove_file(path).expect("test file should be removed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_system_workspaces_are_protected_without_blocking_home_projects() {
        assert!(super::is_protected_linux_workspace(Path::new("/etc")));
        assert!(super::is_protected_linux_workspace(Path::new("/usr/local")));
        assert!(!super::is_protected_linux_workspace(Path::new(
            "/home/user/project"
        )));
        assert!(!super::is_protected_linux_workspace(Path::new(
            "/tmp/project"
        )));
    }

    #[test]
    fn sizing_report_marks_partial_results_and_overflow() {
        let mut report = DirectorySizeReport {
            bytes: u64::MAX,
            errors: 0,
        };

        add_file_size(&mut report, 1);

        assert_eq!(report.bytes, u64::MAX);
        assert_eq!(report.errors, 1);
        assert!(!report.is_complete());
    }

    #[test]
    fn cancellable_sizing_stops_before_walking_when_requested() {
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let result = super::directory_size_report_with_cancel(Path::new("missing"), &cancelled);

        assert!(result.unwrap().is_none());
    }

    fn unique_test_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "uncached-core-file-identity-{}",
            std::process::id()
        ))
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::file_identity;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    #[test]
    fn file_identity_reads_real_directory_identity() {
        let path = fixture_directory("stable");
        let first = file_identity(&path).expect("directory should have identity");
        let second = file_identity(&path).expect("directory identity should remain readable");

        assert_eq!(first, second);
        cleanup_fixture(path);
    }

    #[test]
    fn file_identity_returns_none_for_missing_directory() {
        let path = fixture_path("missing");

        assert_eq!(file_identity(&path), None);
    }

    #[test]
    fn file_identity_returns_none_when_parent_is_a_reparse_point() {
        let real = fixture_path("reparse-real");
        let link = fixture_path("reparse-link");
        let target = link.join("target");
        fs::create_dir_all(real.join("target")).expect("real target should be created");
        let junction_status = Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&link)
            .arg(&real)
            .status()
            .expect("junction command should start");
        if !junction_status.success() {
            fs::remove_dir_all(real).expect("real fixture should be removed");
            panic!("junction creation unsupported: mklink /J exited with {junction_status}");
        }

        assert_eq!(file_identity(&target), None);

        fs::remove_dir(link).expect("reparse fixture should be removed");
        fs::remove_dir_all(real).expect("real fixture should be removed");
    }

    fn fixture_directory(label: &str) -> PathBuf {
        let path = fixture_path(label);
        fs::create_dir(&path).expect("fixture directory should be created");
        path
    }

    fn fixture_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "uncached-core-file-identity-{label}-{}",
            std::process::id()
        ))
    }

    fn cleanup_fixture(path: PathBuf) {
        fs::remove_dir(path).expect("fixture directory should be removed");
    }
}

/// True when `parent` directly contains a regular file named `name`. Uses
/// `symlink_metadata`, so a symlinked marker file never confirms a deletion
/// context: `is_file` is false for the link itself.
pub fn has_marker_file(parent: &Path, name: &str) -> bool {
    fs::symlink_metadata(parent.join(name)).is_ok_and(|metadata| metadata.is_file())
}

/// True when `parent` directly contains an entry whose extension is `extension`.
///
/// Some ecosystems name the project file after the project rather than after
/// the tool: an Xcode project is `Whatever.xcodeproj` and a .NET one is
/// `Whatever.csproj`, so there is no fixed name to look for. Unlike
/// `has_marker_file` this accepts directories, because `.xcodeproj` and
/// `.xcworkspace` are bundles.
///
/// The comparison is case-insensitive: the filesystems these markers live on
/// usually are, and a project saved as `.CSPROJ` is the same project.
pub fn has_marker_with_extension(parent: &Path, extension: &str) -> bool {
    let Ok(entries) = fs::read_dir(parent) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .is_some_and(|found| found.eq_ignore_ascii_case(extension))
    })
}

pub fn is_within(root: &Path, target: &Path) -> bool {
    target != root && target.starts_with(root)
}

/// Rejects paths that must never be deleted. Callers pass an already canonical
/// path — the cleanup policy canonicalizes once — so this resolves nothing and
/// performs no filesystem access of its own.
pub fn is_safe_to_delete(canonical: &Path) -> bool {
    canonical.parent().is_some() && !is_critical_directory(canonical)
}

fn is_critical_directory(path: &Path) -> bool {
    #[cfg(windows)]
    {
        let normalized = path.to_string_lossy().to_lowercase();
        let critical = [
            std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string()),
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string()),
            std::env::var("ProgramFiles(x86)")
                .unwrap_or_else(|_| r"C:\Program Files (x86)".to_string()),
        ];
        critical.iter().any(|value| {
            let value = value.trim_end_matches(['\\', '/']).to_lowercase();
            normalized == value || normalized.starts_with(&(value + "\\"))
        })
    }
    #[cfg(not(windows))]
    {
        matches!(
            path.to_str(),
            Some("/usr" | "/bin" | "/sbin" | "/etc" | "/var" | "/home" | "/root" | "/opt")
        )
    }
}
