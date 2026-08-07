use crate::error::{CoreError, CoreResult};
use crate::model::FileIdentity;
use jwalk::{Parallelism, WalkDir};
use std::fs;
use std::path::{Path, PathBuf};

pub fn canonical_workspace(root: &Path) -> CoreResult<PathBuf> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| CoreError::InvalidWorkspace(root.display().to_string()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CoreError::InvalidWorkspace(root.display().to_string()));
    }
    fs::canonicalize(root).map_err(CoreError::from)
}

pub fn is_protected_directory(name: &str) -> bool {
    name.eq_ignore_ascii_case("$RECYCLE.BIN")
        || name.eq_ignore_ascii_case("System Volume Information")
}

pub fn directory_size(root: &Path) -> CoreResult<u64> {
    let report = directory_size_report(root)?;
    if report.errors > 0 {
        return Err(CoreError::InvalidTarget(format!(
            "size traversal encountered {} errors",
            report.errors
        )));
    }
    Ok(report.bytes)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirectorySizeReport {
    pub bytes: u64,
    pub errors: u64,
}

pub fn directory_size_report(root: &Path) -> CoreResult<DirectorySizeReport> {
    let workers = std::thread::available_parallelism()
        .map(|count| count.get().clamp(1, 8))
        .unwrap_or(1);
    WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(Parallelism::RayonNewPool(workers))
        .into_iter()
        .try_fold(DirectorySizeReport::default(), |mut report, entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    report.errors += 1;
                    return Ok(report);
                }
            };
            match entry.metadata() {
                Ok(metadata) if metadata.is_file() => {
                    report.bytes = report.bytes.saturating_add(metadata.len());
                }
                Ok(_) => {}
                Err(_) => report.errors += 1,
            }
            Ok(report)
        })
}

pub fn file_identity(path: &Path) -> Option<FileIdentity> {
    let metadata = fs::symlink_metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(FileIdentity {
            first: metadata.dev(),
            second: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

pub fn read_entries(path: &Path) -> CoreResult<Vec<fs::DirEntry>> {
    fs::read_dir(path)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CoreError::from)
}

pub fn is_within(root: &Path, target: &Path) -> bool {
    target != root && target.starts_with(root)
}

pub fn is_safe_to_delete(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    let Ok(canonical) = fs::canonicalize(path) else {
        return false;
    };
    canonical.parent().is_some() && !is_critical_directory(&canonical)
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
