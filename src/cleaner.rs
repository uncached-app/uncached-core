use crate::error::{CoreError, CoreResult};
use crate::filesystem::{
    canonical_workspace, directory_size_report, file_identity, is_safe_to_delete, is_within,
};
use crate::model::{CleanReport, ScanTarget};
use crate::scanner::ProviderSet;
use std::fs;
use std::path::{Path, PathBuf};

pub fn validate_clean_target(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<PathBuf> {
    let root = canonical_workspace(root)?;
    let scanner = providers
        .scanner_for(target.category.clone())
        .ok_or_else(|| CoreError::InvalidTarget("missing provider".into()))?;
    let expected = target
        .identity
        .ok_or_else(|| CoreError::InvalidTarget("target filesystem identity unavailable".into()))?;
    let actual = file_identity(&target.path)
        .ok_or_else(|| CoreError::InvalidTarget("target filesystem identity unavailable".into()))?;
    if expected != actual {
        return Err(CoreError::InvalidTarget(
            "target filesystem identity changed".into(),
        ));
    }
    let metadata = fs::symlink_metadata(&target.path)
        .map_err(|_| CoreError::InvalidTarget("target does not exist".into()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CoreError::InvalidTarget(
            "target must be a real directory".into(),
        ));
    }
    if target.path.ancestors().any(|ancestor| {
        fs::symlink_metadata(ancestor).is_ok_and(|metadata| metadata.file_type().is_symlink())
    }) {
        return Err(CoreError::InvalidTarget(
            "target has a symlinked ancestor".into(),
        ));
    }
    let canonical = fs::canonicalize(&target.path)?;
    let name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CoreError::InvalidTarget("target has invalid basename".into()))?;
    if !scanner.matches(name) {
        return Err(CoreError::InvalidTarget(
            "target name does not match provider".into(),
        ));
    }
    if !scanner.confirm_context(&canonical) {
        return Err(CoreError::InvalidTarget("target context is invalid".into()));
    }
    if !is_within(&root, &canonical) || !is_safe_to_delete(&canonical) {
        return Err(CoreError::InvalidTarget(
            "target is outside safe workspace".into(),
        ));
    }
    Ok(canonical)
}

pub fn clean_to_trash(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<CleanReport> {
    clean_to_trash_with_delete(root, providers, target, |path| {
        trash::delete(path).map_err(|error| error.to_string())
    })
}

pub fn clean_permanently(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<CleanReport> {
    clean_permanently_with_delete_and_size(
        root,
        providers,
        target,
        |path| directory_size_report(path).map(|report| (report.bytes, report.errors > 0)),
        |path| fs::remove_dir_all(path).map_err(CoreError::from),
    )
}

pub fn clean_permanently_with_delete_and_size<S, F>(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
    mut size: S,
    delete: F,
) -> CoreResult<CleanReport>
where
    S: FnMut(&Path) -> CoreResult<(u64, bool)>,
    F: FnOnce(&Path) -> CoreResult<()>,
{
    let canonical = validate_clean_target(root, providers, target)?;
    let (freed_bytes, size_has_errors) = match size(&canonical) {
        Ok((bytes, has_errors)) => (bytes, has_errors),
        Err(_) => (0, true),
    };
    let canonical = validate_clean_target(root, providers, target)?;
    delete(&canonical)?;
    Ok(CleanReport {
        freed_bytes,
        size_has_errors,
    })
}

pub fn clean_to_trash_with_delete<F>(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
    delete: F,
) -> CoreResult<CleanReport>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    clean_to_trash_with_delete_and_size(
        root,
        providers,
        target,
        |path| directory_size_report(path).map(|report| (report.bytes, report.errors > 0)),
        delete,
    )
}

pub fn clean_to_trash_with_delete_and_size<S, F>(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
    mut size: S,
    delete: F,
) -> CoreResult<CleanReport>
where
    S: FnMut(&Path) -> CoreResult<(u64, bool)>,
    F: FnOnce(&Path) -> Result<(), String>,
{
    let canonical = validate_clean_target(root, providers, target)?;
    let (freed_bytes, size_has_errors) = match size(&canonical) {
        Ok((bytes, has_errors)) => (bytes, has_errors),
        Err(_) => (0, true),
    };
    let canonical = validate_clean_target(root, providers, target)?;
    match delete(&canonical) {
        Ok(()) => Ok(CleanReport {
            freed_bytes,
            size_has_errors,
        }),
        Err(error) => {
            if error.to_lowercase().contains("too large")
                || error.to_lowercase().contains("lixeira")
            {
                Err(CoreError::RecycleBinTooLarge)
            } else {
                Err(CoreError::Trash(error))
            }
        }
    }
}
