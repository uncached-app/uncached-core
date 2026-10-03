use crate::error::{CoreError, CoreResult};
use std::fs;
use std::path::{Path, PathBuf};

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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::path::Path;

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
}
