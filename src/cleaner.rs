use crate::error::{CoreError, CoreResult};
use crate::filesystem::{
    canonical_workspace, directory_size_report, file_identity, is_safe_to_delete, is_within,
};
use crate::model::{CleanReport, ScanTarget};
use crate::scanner::ProviderSet;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(all(test, any(unix, windows)))]
use std::sync::{Mutex, OnceLock};

#[cfg(all(test, any(unix, windows)))]
type SecureTargetHook = Box<dyn FnOnce() + Send>;

#[cfg(all(test, any(unix, windows)))]
static SECURE_TARGET_HOOK: OnceLock<Mutex<Option<SecureTargetHook>>> = OnceLock::new();

pub fn validate_clean_target(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<PathBuf> {
    let policy = validate_clean_policy(root, providers, target)?;
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
    Ok(policy.target)
}

struct CleanPolicy {
    root: PathBuf,
    target: PathBuf,
}

fn validate_clean_policy(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<CleanPolicy> {
    let root = canonical_workspace(root)?;
    let scanner = providers
        .scanner_for(target.category.clone())
        .ok_or_else(|| CoreError::InvalidTarget("missing provider".into()))?;
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
    Ok(CleanPolicy {
        root,
        target: canonical,
    })
}

pub fn clean_to_trash(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<CleanReport> {
    let policy = validate_clean_policy(root, providers, target)?;
    let secure_target = crate::secure_cleanup::open_secure_target(
        &policy.root,
        providers,
        &canonical_target(target, &policy),
    )?;
    #[cfg(all(test, any(unix, windows)))]
    run_secure_target_hook();
    secure_target.move_to_trash().map(|()| CleanReport {
        freed_bytes: 0,
        size_has_errors: true,
    })
}

pub fn clean_permanently(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<CleanReport> {
    let policy = validate_clean_policy(root, providers, target)?;
    let secure_target = crate::secure_cleanup::open_secure_target(
        &policy.root,
        providers,
        &canonical_target(target, &policy),
    )?;
    #[cfg(all(test, any(unix, windows)))]
    run_secure_target_hook();
    secure_target.delete_permanently()?;
    Ok(CleanReport {
        freed_bytes: 0,
        size_has_errors: true,
    })
}

fn canonical_target(target: &ScanTarget, policy: &CleanPolicy) -> ScanTarget {
    let mut canonical = target.clone();
    canonical.path = policy.target.clone();
    canonical
}

#[cfg(all(test, any(unix, windows)))]
fn install_secure_target_hook(hook: impl FnOnce() + Send + 'static) {
    *SECURE_TARGET_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap() = Some(Box::new(hook));
}

#[cfg(all(test, any(unix, windows)))]
fn run_secure_target_hook() {
    let hook = SECURE_TARGET_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .take();
    if let Some(hook) = hook {
        hook();
    }
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

#[cfg(test)]
mod tests {
    use super::clean_permanently;
    #[cfg(any(unix, windows))]
    use super::install_secure_target_hook;
    use crate::filesystem::file_identity;
    use crate::{Category, CoreError, ProviderSet, ScanTarget};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    #[cfg(any(unix, windows))]
    fn public_cleanup_cannot_delete_same_basename_replacement() {
        let root = fixture();
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(target_path.join("original.txt"), b"original").unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let identity = file_identity(&target_path).unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 8)
            .with_identity(Some(identity));
        let replacement = target_path.clone();
        let original = project.join("node_modules-original");

        let hook_replacement = fs::canonicalize(&replacement).unwrap();
        let hook_original = original.clone();
        install_secure_target_hook(move || {
            #[cfg(unix)]
            {
                fs::rename(&hook_replacement, &hook_original).unwrap();
                fs::create_dir(&hook_replacement).unwrap();
                fs::write(hook_replacement.join("replacement.txt"), b"replacement").unwrap();
            }
            #[cfg(windows)]
            {
                let error = fs::rename(&hook_replacement, &hook_original).unwrap_err();
                assert_eq!(error.raw_os_error(), Some(5));
            }
        });

        let result = clean_permanently(&root, &ProviderSet::free(), &target);

        #[cfg(windows)]
        {
            result.unwrap();
            assert!(!replacement.exists());
            assert!(!original.exists());
        }
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
        {
            assert!(matches!(
                result,
                Err(CoreError::SecureOperationUnsupported { .. })
            ));
            assert!(original.join("original.txt").exists());
            assert!(replacement.join("replacement.txt").exists());
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(windows)]
    fn public_cleanup_rejects_replaced_target_without_deleting_replacement() {
        let root = fixture();
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(target_path.join("original.txt"), b"original").unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 8)
            .with_identity(file_identity(&target_path));
        let original = project.join("node_modules-original");
        fs::rename(&target_path, &original).unwrap();
        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("replacement.txt"), b"replacement").unwrap();

        let result = clean_permanently(&root, &ProviderSet::free(), &target);

        assert!(matches!(
            result,
            Err(CoreError::TargetIdentityChanged { .. })
        ));
        assert!(target_path.join("replacement.txt").exists());
        assert!(original.join("original.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_trash_cleanup_reports_typed_unavailability_without_mutation() {
        let root = fixture();
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(target_path.join("child.txt"), b"content").unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 7)
            .with_identity(file_identity(&target_path));

        let result = super::clean_to_trash(&root, &ProviderSet::free(), &target);

        assert!(
            matches!(result, Err(CoreError::SecureTrashUnavailable)),
            "unexpected result: {result:?}"
        );
        assert!(target_path.join("child.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_permanent_cleanup_uses_canonical_workspace_root() {
        let root = fixture();
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(target_path.join("child.txt"), b"content").unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 7)
            .with_identity(file_identity(&target_path));

        let result = super::clean_permanently(
            &root.join("project").join(".."),
            &ProviderSet::free(),
            &target,
        );

        #[cfg(windows)]
        {
            result.unwrap();
            assert!(!target_path.exists());
        }
        #[cfg(not(windows))]
        assert!(matches!(
            result,
            Err(CoreError::SecureOperationUnsupported { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_trash_cleanup_uses_canonical_workspace_root() {
        let root = fixture();
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(target_path.join("child.txt"), b"content").unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 7)
            .with_identity(file_identity(&target_path));

        let result = super::clean_to_trash(
            &root.join("project").join(".."),
            &ProviderSet::free(),
            &target,
        );

        assert!(matches!(result, Err(CoreError::SecureTrashUnavailable)));
        assert_eq!(fs::read(target_path.join("child.txt")).unwrap(), b"content");
        let _ = fs::remove_dir_all(root);
    }

    fn fixture() -> PathBuf {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed").replace(':', "-");
        let root = std::env::temp_dir().join(format!(
            "uncached-core-cleaner-{}-{}",
            std::process::id(),
            thread_name
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
