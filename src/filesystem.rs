use crate::error::{CoreError, CoreResult};
use crate::model::FileIdentity;
use crate::parallel::worker_pool;
use jwalk::{Parallelism, WalkDir};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The canonical form of a path, in the spelling this crate uses everywhere.
///
/// One function rather than a call to `dunce` at each site, because the whole
/// point is that there is a single answer. Paths here are compared against each
/// other — an ignore rule against a scan result, a catalogued location against
/// a candidate, a test's expectation against what a scan reported — and two
/// spellings of one directory that stop matching is a worse fault than an ugly
/// one. Changing this in three files out of five is exactly how that fault was
/// introduced, twice.
///
/// Public so the integration tests reach it too. They are separate crates and
/// cannot see this crate's dependencies, so without this they would go on
/// calling `fs::canonicalize` and disagreeing with the code under test — which
/// is what they did.
///
/// On Windows `fs::canonicalize` returns the extended-length form,
/// `\\?\C:\work`. `dunce` gives the ordinary one whenever it exists and the
/// verbatim one when it is genuinely required: past 260 characters, or on a
/// reserved name. Everywhere else this is `fs::canonicalize` exactly.
pub fn canonical(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

pub fn canonical_workspace(root: &Path) -> CoreResult<PathBuf> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| CoreError::InvalidWorkspace(root.display().to_string()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CoreError::InvalidWorkspace(root.display().to_string()));
    }
    let canonical = crate::filesystem::canonical(root).map_err(CoreError::from)?;
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
    /// Newest modification time in the tree, in seconds since the Unix epoch.
    ///
    /// Answers how long ago a cache was last written to, which is the only
    /// cheap signal there is for whether anything still depends on it. Taken
    /// from files alone: a directory's own timestamp changes when its children
    /// are listed on some systems, and a tree with no readable file has no
    /// answer rather than a wrong one.
    pub newest_modified: Option<i64>,
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

/// Keeps the newest of the times seen. A clock that went backwards before the
/// epoch is recorded as negative rather than discarded, so the comparison stays
/// a comparison of instants.
fn note_modified(report: &mut DirectorySizeReport, metadata: &std::fs::Metadata) {
    let Ok(modified) = metadata.modified() else {
        return;
    };
    let seconds = match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => since.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    };
    if report.newest_modified.is_none_or(|newest| seconds > newest) {
        report.newest_modified = Some(seconds);
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
    directory_size_report_skipping(root, &[], cancelled)
}

/// The same, without descending into the subtrees named.
///
/// Measuring a project used to walk it whole and then walk each artifact
/// directory again for its own figure, so `node_modules` and `target` were
/// visited twice — and in a typical project those are ninety-eight per cent of
/// the files. Sizing the artifacts once and the rest without them gives both
/// numbers for one pass over each file.
///
/// The doubling costs almost nothing on a warm macOS cache and is what makes
/// this take minutes on Windows, where a real-time scanner inspects every file
/// as it is opened. The waste was on every platform; only one of them notices.
///
/// `skip` holds directories, compared by exact path against what the walk is
/// about to descend into. Callers pass paths from the same traversal, so they
/// already agree on spelling.
pub fn directory_size_report_skipping(
    root: &Path,
    skip: &[PathBuf],
    cancelled: &AtomicBool,
) -> CoreResult<Option<DirectorySizeReport>> {
    let skip: std::collections::HashSet<PathBuf> = skip.iter().cloned().collect();
    WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(size_parallelism())
        .process_read_dir(move |_, _, _, children| {
            // Pruned here rather than filtered afterwards: the point is not to
            // read what is inside them.
            children.retain(|child| match child {
                Ok(entry) => !skip.contains(&entry.path()),
                Err(_) => true,
            });
        })
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
                    note_modified(&mut report, &metadata);
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

#[cfg(test)]
mod skipping_tests {
    use super::{directory_size_report, directory_size_report_skipping};
    use std::fs;
    use std::sync::atomic::AtomicBool;

    /// The property the optimisation rests on: measuring the artifacts once and
    /// the rest without them adds up to measuring everything. If it ever does
    /// not, a project's total silently loses whatever fell through the gap —
    /// which is worse than the slowness this replaced.
    #[test]
    fn what_is_skipped_plus_what_is_skipping_it_is_the_whole() {
        let root = std::env::temp_dir().join(format!("uncached-skip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let artifact = root.join("node_modules");
        fs::create_dir_all(artifact.join("deep").join("deeper")).expect("fixture");
        fs::create_dir_all(root.join("src")).expect("fixture");
        fs::write(root.join("src").join("main.rs"), vec![b'a'; 500]).expect("fixture");
        fs::write(root.join("README.md"), vec![b'b'; 300]).expect("fixture");
        fs::write(artifact.join("package.json"), vec![b'c'; 700]).expect("fixture");
        fs::write(
            artifact.join("deep").join("deeper").join("blob"),
            vec![b'd'; 1100],
        )
        .expect("fixture");

        let whole = directory_size_report(&root).expect("whole");
        let without = directory_size_report_skipping(
            &root,
            std::slice::from_ref(&artifact),
            &AtomicBool::new(false),
        )
        .expect("without")
        .expect("not cancelled");
        let only = directory_size_report(&artifact).expect("only");

        assert_eq!(
            without.bytes, 800,
            "the source files, and nothing under the artifact"
        );
        assert_eq!(only.bytes, 1800);
        assert_eq!(
            without.bytes + only.bytes,
            whole.bytes,
            "one pass over each file has to reach the same total as two"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// Skipping a path that is not there changes nothing, which is what lets a
    /// caller pass targets that vanished between the walk and the sizing.
    #[test]
    fn a_skip_that_matches_nothing_is_not_a_smaller_total() {
        let root = std::env::temp_dir().join(format!("uncached-skip-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("fixture");
        fs::write(root.join("file"), vec![b'x'; 42]).expect("fixture");

        let whole = directory_size_report(&root).expect("whole");
        let skipped = directory_size_report_skipping(
            &root,
            &[root.join("does-not-exist")],
            &AtomicBool::new(false),
        )
        .expect("skipped")
        .expect("not cancelled");

        assert_eq!(skipped.bytes, whole.bytes);
        let _ = fs::remove_dir_all(&root);
    }
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

    /// The newest file in the tree is what says how long ago the cache was
    /// written to, and a cache nothing has touched in months is the one a user
    /// can decide about without opening it.
    #[test]
    fn sizing_reports_the_newest_file_in_the_tree() {
        let root = test_tree("newest");
        fs::create_dir_all(root.join("nested")).expect("fixture should be created");
        fs::write(root.join("old.bin"), b"old").expect("file should be written");
        fs::write(root.join("nested/new.bin"), b"new").expect("file should be written");

        let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(root.join("old.bin"))
            .expect("file should open")
            .set_modified(long_ago)
            .expect("modification time should be settable");

        let report = super::directory_size_report(&root).expect("fixture should be sized");
        let newest = report
            .newest_modified
            .expect("a tree with files has an age");

        assert!(
            newest > 1_000_000_000,
            "the newest file wins over the oldest one"
        );

        let empty = test_tree("empty");
        fs::create_dir_all(&empty).expect("fixture should be created");
        assert_eq!(
            super::directory_size_report(&empty)
                .expect("fixture should be sized")
                .newest_modified,
            None,
            "a tree with no readable file has no answer to give"
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&empty);
    }

    #[test]
    fn sizing_report_marks_partial_results_and_overflow() {
        let mut report = DirectorySizeReport {
            bytes: u64::MAX,
            errors: 0,
            newest_modified: None,
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

    /// A directory of its own per case: `unique_test_path` is one path for the
    /// whole module, and another test leaves a file standing there.
    fn test_tree(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("uncached-core-size-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
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

/// True when `parent` directly contains a real directory named `name`.
///
/// Some ecosystems are identified by the directories they require rather than
/// by a file: a Unity project is the `Assets` and `ProjectSettings` pair, and
/// nothing else on disk carries that signature. `symlink_metadata` keeps a
/// symlinked directory from standing in for the real one, for the same reason
/// as `has_marker_file`.
pub fn has_marker_directory(parent: &Path, name: &str) -> bool {
    fs::symlink_metadata(parent.join(name)).is_ok_and(|metadata| metadata.is_dir())
}

/// True when `parent` directly contains any of `names` as a regular file.
pub fn has_any_marker_file(parent: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| has_marker_file(parent, name))
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
