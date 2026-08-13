use crate::error::{CoreError, CoreResult};
use crate::filesystem::{canonical_workspace, file_identity, is_safe_to_delete, is_within};
use crate::model::{CleanReport, ScanTarget};
use crate::scanner::ProviderSet;
use crate::secure_cleanup::{open_secure_target, CleanupIntent};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(all(test, windows))]
use std::cell::RefCell;

#[cfg(all(test, windows))]
type SecureTargetHook = Box<dyn FnOnce() + Send>;

#[cfg(all(test, windows))]
thread_local! {
    static SECURE_TARGET_HOOK: RefCell<Option<SecureTargetHook>> = RefCell::new(None);
}

/// Validates a target against its workspace, provider and recorded identity.
/// Returns the canonical target path that every later step must use.
pub fn validate_clean_target(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<PathBuf> {
    validate_clean_policy(root, providers, target).map(|policy| policy.target)
}

struct CleanPolicy {
    root: PathBuf,
    target: PathBuf,
}

/// Policy owns every pre-deletion decision: workspace containment, provider
/// agreement, and the recorded-versus-current identity check. Backends add
/// handle-level enforcement on top; they never re-decide policy.
fn validate_clean_policy(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<CleanPolicy> {
    let root = canonical_workspace(root)?;
    let scanner = providers
        .scanner_for(&target.category)
        .ok_or(CoreError::ProviderUnavailable)?;
    let metadata = fs::symlink_metadata(&target.path).map_err(|_| CoreError::TargetMissing {
        path: target.path.clone(),
    })?;
    if metadata.file_type().is_symlink() {
        return Err(CoreError::TargetReparsePoint {
            path: target.path.clone(),
        });
    }
    if !metadata.is_dir() {
        return Err(CoreError::TargetNotDirectory {
            path: target.path.clone(),
        });
    }
    if target.path.ancestors().any(|ancestor| {
        let is_symlink =
            fs::symlink_metadata(ancestor).is_ok_and(|metadata| metadata.file_type().is_symlink());
        is_symlink
            && fs::canonicalize(ancestor)
                .is_ok_and(|canonical_ancestor| is_within(&root, &canonical_ancestor))
    }) {
        return Err(CoreError::TargetReparsePoint {
            path: target.path.clone(),
        });
    }
    let canonical = fs::canonicalize(&target.path)?;
    let name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CoreError::TargetNameUnsupported {
            path: canonical.clone(),
        })?;
    if !scanner.matches(name) {
        return Err(CoreError::TargetProviderMismatch { path: canonical });
    }
    if !scanner.confirm_context(&canonical) {
        return Err(CoreError::TargetContextInvalid { path: canonical });
    }
    if !is_within(&root, &canonical) || !is_safe_to_delete(&canonical) {
        return Err(CoreError::TargetOutsideWorkspace { path: canonical });
    }
    // Checked again here, not only during the walk. A target found before the
    // rule existed is still in the caller's list, and the walk is not what
    // authorizes a deletion — this is.
    if providers.excludes(&canonical) {
        return Err(CoreError::TargetIgnoredByRule { path: canonical });
    }
    verify_recorded_identity(target, &canonical)?;
    Ok(CleanPolicy {
        root,
        target: canonical,
    })
}

fn verify_recorded_identity(target: &ScanTarget, canonical: &Path) -> CoreResult<()> {
    let expected = target
        .identity
        .ok_or_else(|| CoreError::TargetIdentityUnavailable {
            path: canonical.to_path_buf(),
        })?;
    let actual = file_identity(canonical).ok_or_else(|| CoreError::TargetIdentityUnavailable {
        path: canonical.to_path_buf(),
    })?;
    if expected != actual {
        return Err(CoreError::TargetIdentityChanged {
            path: canonical.to_path_buf(),
        });
    }
    Ok(())
}

pub fn clean_to_trash(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<CleanReport> {
    let policy = validate_clean_policy(root, providers, target)?;
    let canonical = canonical_target(target, &policy);
    let secure_target = open_secure_target(&policy.root, &canonical, CleanupIntent::Trash)?;
    #[cfg(all(test, windows))]
    run_secure_target_hook();
    // Re-run policy after the target is pinned: the Trash backend releases its
    // verification handles before the Shell call, so the window is revalidated.
    validate_clean_policy(&policy.root, providers, &canonical)?;
    secure_target
        .move_to_trash()
        .map(|()| clean_report_for(target))
}

pub fn clean_permanently(
    root: &Path,
    providers: &ProviderSet,
    target: &ScanTarget,
) -> CoreResult<CleanReport> {
    let policy = validate_clean_policy(root, providers, target)?;
    let canonical = canonical_target(target, &policy);
    let secure_target = open_secure_target(&policy.root, &canonical, CleanupIntent::Permanent)?;
    #[cfg(all(test, windows))]
    run_secure_target_hook();
    secure_target.delete_permanently()?;
    Ok(clean_report_for(target))
}

/// Secure cleanup does not walk the target after deletion. It reports the size
/// already measured during scanning instead.
fn clean_report_for(target: &ScanTarget) -> CleanReport {
    CleanReport {
        freed_bytes: target.size_bytes,
        size_pending: target.size_pending,
        size_has_errors: target.size_has_errors,
    }
}

fn canonical_target(target: &ScanTarget, policy: &CleanPolicy) -> ScanTarget {
    let mut canonical = target.clone();
    canonical.path = policy.target.clone();
    canonical
}

#[cfg(all(test, windows))]
fn install_secure_target_hook(hook: impl FnOnce() + Send + 'static) {
    SECURE_TARGET_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, windows))]
fn run_secure_target_hook() {
    let hook = SECURE_TARGET_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
mod tests {
    use super::{clean_permanently, clean_to_trash};
    use crate::filesystem::file_identity;
    use crate::{Category, CoreError, ProviderSet, ScanTarget};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn public_trash_cleanup_reports_typed_result_without_partial_mutation() {
        let root = fixture();
        let _cleanup = CleanupGuard(root.clone());
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(target_path.join("child.txt"), b"content").unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 7)
            .with_identity(file_identity(&target_path));

        let result = clean_to_trash(&root, &ProviderSet::free(), &target);

        #[cfg(windows)]
        match result {
            Ok(report) => {
                assert!(!target_path.exists());
                assert_eq!(report.freed_bytes, 7);
                assert!(!report.size_pending);
                assert!(!report.size_has_errors);
            }
            Err(CoreError::SecureTrashUnavailable)
            | Err(CoreError::SecureOperationUnsupported { .. }) => {
                eprintln!("skipping: Windows Trash backend unavailable or unsupported");
                assert!(target_path.exists());
            }
            Err(error) => panic!("Windows Trash cleanup failed: {error:?}"),
        }
        #[cfg(all(unix, not(target_os = "android")))]
        {
            let report = result.unwrap();
            assert_eq!(report.freed_bytes, 7);
            assert!(!report.size_pending);
            assert!(!target_path.exists());
        }
        #[cfg(not(any(windows, all(unix, not(target_os = "android")))))]
        {
            assert!(matches!(result, Err(CoreError::SecureTrashUnavailable)));
            assert!(target_path.is_dir());
            assert_eq!(fs::read(target_path.join("child.txt")).unwrap(), b"content");
        }
    }

    #[test]
    fn public_permanent_cleanup_uses_canonical_workspace_root() {
        let root = fixture();
        let _cleanup = CleanupGuard(root.clone());
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(target_path.join("child.txt"), b"content").unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 7)
            .with_identity(file_identity(&target_path));

        let result = clean_permanently(
            &root.join("project").join(".."),
            &ProviderSet::free(),
            &target,
        );

        #[cfg(windows)]
        {
            let report = result.unwrap();
            assert!(!target_path.exists());
            assert_eq!(report.freed_bytes, 7);
            assert!(!report.size_pending);
        }
        #[cfg(all(unix, not(target_os = "android")))]
        {
            let report = result.unwrap();
            assert_eq!(report.freed_bytes, 7);
            assert!(!report.size_pending);
            assert!(!target_path.exists());
        }
        #[cfg(not(any(windows, all(unix, not(target_os = "android")))))]
        {
            assert!(matches!(
                result,
                Err(CoreError::SecureOperationUnsupported { .. })
            ));
            assert_eq!(fs::read(target_path.join("child.txt")).unwrap(), b"content");
        }
    }

    #[test]
    fn cleanup_report_preserves_sizing_errors() {
        let root = fixture();
        let _cleanup = CleanupGuard(root.clone());
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 7)
            .with_identity(file_identity(&target_path))
            .with_size_state(false, true);

        let report = clean_permanently(&root, &ProviderSet::free(), &target).unwrap();

        assert_eq!(report.freed_bytes, 7);
        assert!(!report.size_pending);
        assert!(report.size_has_errors);
    }

    #[test]
    fn policy_rejects_target_whose_recorded_identity_is_missing() {
        let root = fixture();
        let _cleanup = CleanupGuard(root.clone());
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0);

        assert!(matches!(
            clean_permanently(&root, &ProviderSet::free(), &target),
            Err(CoreError::TargetIdentityUnavailable { .. })
        ));
        assert!(target_path.is_dir());
    }

    #[test]
    fn policy_rejects_target_without_a_registered_provider() {
        let root = fixture();
        let _cleanup = CleanupGuard(root.clone());
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path, Category::NodeModules, 0).with_identity(Some(
            crate::FileIdentity {
                volume_serial: 0,
                file_id: [0; 16],
            },
        ));

        assert!(matches!(
            clean_permanently(&root, &ProviderSet::new(), &target),
            Err(CoreError::ProviderUnavailable)
        ));
    }

    #[test]
    fn policy_rejects_missing_target_before_touching_a_backend() {
        let root = fixture();
        let _cleanup = CleanupGuard(root.clone());
        let target = ScanTarget::new(root.join("absent"), Category::NodeModules, 0).with_identity(
            Some(crate::FileIdentity {
                volume_serial: 0,
                file_id: [0; 16],
            }),
        );

        assert!(matches!(
            clean_permanently(&root, &ProviderSet::free(), &target),
            Err(CoreError::TargetMissing { .. })
        ));
    }

    #[test]
    fn policy_rejects_context_that_no_longer_confirms_the_provider() {
        let root = fixture();
        let _cleanup = CleanupGuard(root.clone());
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&target_path));

        assert!(matches!(
            clean_permanently(&root, &ProviderSet::free(), &target),
            Err(CoreError::TargetContextInvalid { .. })
        ));
        assert!(target_path.is_dir());
    }

    #[test]
    fn policy_rejects_target_whose_identity_changed_since_the_scan() {
        let root = fixture();
        let _cleanup = CleanupGuard(root.clone());
        let project = root.join("project");
        let target_path = project.join("node_modules");
        fs::create_dir_all(&target_path).unwrap();
        fs::write(project.join("package.json"), b"{}").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0).with_identity(
            Some(crate::FileIdentity {
                volume_serial: u64::MAX,
                file_id: [0xAB; 16],
            }),
        );

        assert!(matches!(
            clean_permanently(&root, &ProviderSet::free(), &target),
            Err(CoreError::TargetIdentityChanged { .. })
        ));
        assert!(target_path.is_dir());
    }

    struct CleanupGuard(PathBuf);

    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
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
        fs::canonicalize(root).unwrap()
    }

    #[cfg(windows)]
    mod windows {
        use super::super::install_secure_target_hook;
        use super::{fixture, CleanupGuard};
        use crate::filesystem::file_identity;
        use crate::{Category, CoreError, ProviderSet, ScanTarget};
        use std::fs;
        use std::os::windows::ffi::OsStrExt;
        use std::sync::{mpsc, Arc, Mutex};
        use std::time::Duration;

        #[test]
        fn public_cleanup_cannot_delete_same_basename_replacement() {
            let root = fixture();
            let _cleanup = CleanupGuard(root.clone());
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

            let hook_source = {
                let source = project.join("node_modules-replacement");
                fs::create_dir(&source).unwrap();
                fs::canonicalize(source).unwrap()
            };
            let hook_replacement = fs::canonicalize(&replacement).unwrap();
            let (done_tx, done_rx) = mpsc::channel();
            let (stop_tx, stop_rx) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::channel();
            let worker_slot = Arc::new(Mutex::new(None));
            let worker_slot_for_hook = Arc::clone(&worker_slot);
            install_secure_target_hook(move || {
                let worker = std::thread::spawn(move || loop {
                    match rename_replacing(&hook_source, &hook_replacement) {
                        Ok(()) => {
                            let result =
                                fs::write(hook_replacement.join("replacement.txt"), b"replacement")
                                    .map_err(|error| error.to_string());
                            let _ = done_tx.send(result);
                            break;
                        }
                        Err(error) => {
                            if error.raw_os_error() != Some(5) {
                                let _ = done_tx.send(Err(error.to_string()));
                                break;
                            }
                            let _ = ready_tx.send(());
                            if stop_rx.try_recv().is_ok() {
                                break;
                            }
                            std::thread::yield_now();
                        }
                    }
                });
                ready_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("worker should observe retained-handle rename refusal");
                *worker_slot_for_hook.lock().unwrap() = Some(worker);
            });

            let result = super::super::clean_permanently(&root, &ProviderSet::free(), &target);
            let worker_result = done_rx.recv_timeout(Duration::from_secs(5));
            if worker_result.is_err() {
                let _ = stop_tx.send(());
            }
            let worker = worker_slot.lock().unwrap().take().unwrap();
            worker.join().unwrap();
            worker_result.unwrap().unwrap();

            result.unwrap();
            assert_eq!(
                fs::read(replacement.join("replacement.txt")).unwrap(),
                b"replacement"
            );
            assert!(!replacement.join("original.txt").exists());
            assert!(!original.exists());
        }

        #[test]
        fn public_cleanup_rejects_replaced_target_without_deleting_replacement() {
            let root = fixture();
            let _cleanup = CleanupGuard(root.clone());
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

            let result = super::super::clean_permanently(&root, &ProviderSet::free(), &target);

            assert!(matches!(
                result,
                Err(CoreError::TargetIdentityChanged { .. })
            ));
            assert!(target_path.join("replacement.txt").exists());
            assert!(original.join("original.txt").exists());
        }

        #[test]
        fn trash_cleanup_race_preserves_original_and_replacement() {
            use crate::secure_cleanup::install_shell_item_boundary_hook;

            let root = fixture();
            let _cleanup = CleanupGuard(root.clone());
            let project = root.join("project");
            let target_path = project.join("node_modules");
            let archived = project.join("node_modules-original");
            fs::create_dir_all(&target_path).unwrap();
            fs::write(target_path.join("original.txt"), b"original").unwrap();
            fs::write(project.join("package.json"), b"{}").unwrap();
            let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 8)
                .with_identity(file_identity(&target_path));
            let source = project.join("node_modules-replacement");
            fs::create_dir(&source).unwrap();
            fs::write(source.join("replacement.txt"), b"replacement").unwrap();
            let source_for_hook = source.clone();
            let target_for_hook = target_path.clone();
            let archived_for_hook = archived.clone();
            install_shell_item_boundary_hook(move || {
                fs::rename(&target_for_hook, &archived_for_hook).unwrap();
                fs::rename(&source_for_hook, &target_for_hook).unwrap();
            });

            let result = super::super::clean_to_trash(&root, &ProviderSet::free(), &target);

            assert!(
                matches!(
                    result,
                    Err(CoreError::TargetIdentityChanged { .. })
                        | Err(CoreError::SecureTrashPostconditionFailed { .. })
                ),
                "unexpected race result: {result:?}"
            );
            assert_eq!(
                fs::read(archived.join("original.txt")).unwrap(),
                b"original"
            );
            assert_eq!(
                fs::read(target_path.join("replacement.txt")).unwrap(),
                b"replacement"
            );
        }

        fn rename_replacing(
            source: &std::path::Path,
            destination: &std::path::Path,
        ) -> std::io::Result<()> {
            let source: Vec<u16> = source
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let destination: Vec<u16> = destination
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: both paths are NUL-terminated and remain alive for the call.
            let ok = unsafe {
                windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                    source.as_ptr(),
                    destination.as_ptr(),
                    windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING,
                )
            };
            if ok == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }
}
