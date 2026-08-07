use crate::error::{CoreError, CoreResult};
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
    let mut total: u64 = 0;
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let mut entries = read_entries(&directory)?;
        entries.sort_by_key(|left| left.file_name());
        for entry in entries {
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                directories.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
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
