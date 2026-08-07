use crate::error::{CoreError, CoreResult};
use crate::filesystem::{canonical_workspace, directory_size, is_safe_to_delete, is_within};
use crate::model::{Category, CleanReport, ScanTarget};
use crate::scanner::{NodeModulesScanner, Scanner};
use std::fs;
use std::path::{Path, PathBuf};

pub fn validate_clean_target(root: &Path, target: &ScanTarget) -> CoreResult<PathBuf> {
    let root = canonical_workspace(root)?;
    if target.category != Category::NodeModules {
        return Err(CoreError::InvalidTarget("unsupported category".into()));
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
    if canonical.file_name().and_then(|name| name.to_str()) != Some("node_modules") {
        return Err(CoreError::InvalidTarget(
            "target name is not node_modules".into(),
        ));
    }
    let parent = canonical
        .parent()
        .ok_or_else(|| CoreError::InvalidTarget("target has no parent".into()))?;
    let package = parent.join("package.json");
    let package_metadata = fs::symlink_metadata(package)
        .map_err(|_| CoreError::InvalidTarget("parent package.json is not regular".into()))?;
    if !package_metadata.is_file() || package_metadata.file_type().is_symlink() {
        return Err(CoreError::InvalidTarget(
            "parent package.json is not regular".into(),
        ));
    }
    if !NodeModulesScanner.confirm_context(&canonical) {
        return Err(CoreError::InvalidTarget(
            "target is not a safe node_modules context".into(),
        ));
    }
    if !is_within(&root, &canonical) || !is_safe_to_delete(&canonical) {
        return Err(CoreError::InvalidTarget(
            "target is outside safe workspace".into(),
        ));
    }
    Ok(canonical)
}

pub fn clean_to_trash(root: &Path, target: &ScanTarget) -> CoreResult<CleanReport> {
    clean_to_trash_with_delete(root, target, |path| {
        trash::delete(path).map_err(|error| error.to_string())
    })
}

pub fn clean_to_trash_with_delete<F>(
    root: &Path,
    target: &ScanTarget,
    delete: F,
) -> CoreResult<CleanReport>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    let canonical = validate_clean_target(root, target)?;
    let freed_bytes = directory_size(&canonical)?;
    let canonical = validate_clean_target(root, target)?;
    match delete(&canonical) {
        Ok(()) => Ok(CleanReport { freed_bytes }),
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
