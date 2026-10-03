//! Moving a target to the Recycle Bin through `IFileOperation`, and the
//! revalidation around it.

use super::*;

#[cfg(test)]
pub(super) type ShellItemBoundaryHook = Box<dyn FnOnce() + Send>;

#[cfg(test)]
thread_local! {
    static SHELL_ITEM_BOUNDARY_HOOK: RefCell<Option<ShellItemBoundaryHook>> = RefCell::new(None);
    static FORCE_TRASH_ABORT: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn install_shell_item_boundary_hook(hook: impl FnOnce() + Send + 'static) {
    SHELL_ITEM_BOUNDARY_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
pub(super) fn take_shell_item_boundary_hook() -> Option<ShellItemBoundaryHook> {
    SHELL_ITEM_BOUNDARY_HOOK.with(|slot| slot.borrow_mut().take())
}

#[cfg(test)]
pub(crate) fn force_next_trash_abort() {
    FORCE_TRASH_ABORT.with(|abort| abort.set(true));
}

#[cfg(test)]
pub(super) fn take_forced_trash_abort() -> bool {
    FORCE_TRASH_ABORT.with(|abort| abort.replace(false))
}

/// Ancestors are held open only so their handles keep the path pinned while the
/// target is verified; nothing reads the handle back.
#[derive(Debug)]
pub(super) struct Ancestor {
    pub(super) _handle: Handle,
    pub(super) identity: FileIdentity,
    pub(super) path: PathBuf,
}

#[derive(Debug)]
pub(super) struct AncestorSnapshot {
    pub(super) identity: FileIdentity,
    pub(super) path: PathBuf,
}

pub(super) struct ShellTrashInput {
    pub(super) path: PathBuf,
    pub(super) expected_identity: FileIdentity,
    pub(super) ancestor_snapshots: Vec<AncestorSnapshot>,
    #[cfg(test)]
    pub(super) boundary_hook: Option<ShellItemBoundaryHook>,
    #[cfg(test)]
    pub(super) force_aborted_result: bool,
}

pub(super) fn run_trash_on_sta(input: ShellTrashInput) -> CoreResult<()> {
    let ShellTrashInput {
        path,
        expected_identity,
        ancestor_snapshots,
        #[cfg(test)]
        boundary_hook,
        #[cfg(test)]
        force_aborted_result,
    } = input;
    // All validation and Shell calls stay on this dedicated STA. Only owned, Send-safe input
    // crosses the thread boundary; no handle or COM interface is transferred.
    let _apartment = ComApartment::initialize_sta()?;
    for ancestor in &ancestor_snapshots {
        revalidate_shell_ancestor(ancestor, &path)?;
    }
    revalidate_shell_target(&path, expected_identity)?;
    #[cfg(test)]
    if let Some(hook) = boundary_hook {
        hook();
    }
    for ancestor in &ancestor_snapshots {
        revalidate_shell_ancestor(ancestor, &path)?;
    }
    revalidate_shell_target(&path, expected_identity)?;
    let wide = shell_wide_path(&path);
    let mut shell_item = std::ptr::null_mut();
    // SAFETY: wide is NUL-terminated and output pointer is valid storage.
    let result = unsafe {
        SHCreateItemFromParsingName(
            wide.as_ptr(),
            std::ptr::null_mut(),
            &IID_ISHELL_ITEM,
            &mut shell_item,
        )
    };
    if result < 0 {
        return Err(map_trash_error(
            &path,
            "SHCreateItemFromParsingName",
            result,
        ));
    }
    // SAFETY: successful API call initialized a COM interface pointer.
    let shell_item = unsafe { ComPtr::<IShellItemTag>::from_raw(shell_item) };

    let mut file_operation = std::ptr::null_mut();
    // SAFETY: CLSID/IID are valid and output pointer is valid storage.
    let result = unsafe {
        CoCreateInstance(
            &FileOperation,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IFILE_OPERATION,
            &mut file_operation,
        )
    };
    if result < 0 {
        return Err(map_trash_error(&path, "CoCreateInstance", result));
    }
    // SAFETY: successful API call initialized a COM interface pointer.
    let file_operation = unsafe { ComPtr::<IFileOperationTag>::from_raw(file_operation) };
    let vtable = unsafe { *(file_operation.ptr as *mut *const IFileOperationVtbl) };
    let flags = FOF_ALLOWUNDO | FOF_NO_UI;
    // SAFETY: file_operation and vtable are valid COM objects; flags are documented ABI value.
    let result = unsafe { ((*vtable).set_operation_flags)(file_operation.ptr, flags) };
    if result < 0 {
        return Err(map_trash_error(&path, "SetOperationFlags", result));
    }
    let (sink, evidence) = ProgressSink::allocate(&path, shell_item.ptr);
    let mut cookie = 0;
    // SAFETY: sink is a valid IFileOperationProgressSink ABI pointer and cookie is writable.
    unsafe { progress_sink_add_ref(sink) };
    unsafe { progress_sink_release(sink) };
    let result = unsafe { ((*vtable).advise)(file_operation.ptr, sink, &mut cookie) };
    // Release sink's construction reference. Successful Advise retains its own reference.
    unsafe { progress_sink_release(sink) };
    if result < 0 {
        return Err(map_trash_error(&path, "Advise", result));
    }

    let operation_result = (|| {
        #[cfg(test)]
        if !force_aborted_result {
            // SAFETY: shell_item and progress subscription are valid COM objects.
            let result = unsafe {
                ((*vtable).delete_item)(file_operation.ptr, shell_item.ptr, std::ptr::null_mut())
            };
            if result < 0 {
                return Err(map_trash_error(&path, "DeleteItem", result));
            }
        }
        #[cfg(not(test))]
        {
            // SAFETY: shell_item and progress subscription are valid COM objects.
            let result = unsafe {
                ((*vtable).delete_item)(file_operation.ptr, shell_item.ptr, std::ptr::null_mut())
            };
            if result < 0 {
                return Err(map_trash_error(&path, "DeleteItem", result));
            }
        }
        // SAFETY: queued operation belongs to valid file_operation object.
        #[cfg(test)]
        let result = if force_aborted_result {
            0
        } else {
            unsafe { ((*vtable).perform_operations)(file_operation.ptr) }
        };
        #[cfg(not(test))]
        let result = unsafe { ((*vtable).perform_operations)(file_operation.ptr) };
        if result < 0 {
            return Err(map_trash_error(&path, "PerformOperations", result));
        }
        let mut aborted = 0;
        // SAFETY: output pointer is valid and object remains alive.
        let result =
            unsafe { ((*vtable).get_any_operations_aborted)(file_operation.ptr, &mut aborted) };
        if result < 0 {
            return Err(map_trash_error(&path, "GetAnyOperationsAborted", result));
        }
        #[cfg(test)]
        if force_aborted_result {
            aborted = 1;
        }
        map_aborted_operations(&path, aborted != 0)?;
        let evidence = evidence
            .lock()
            .map_err(|_| CoreError::SecureTrashPostconditionFailed { path: path.clone() })?;
        classify_trash_progress(&path, &evidence)
    })();

    // SAFETY: file_operation remains alive and cookie came from Advise.
    let unadvise_result = unsafe { ((*vtable).unadvise)(file_operation.ptr, cookie) };
    if unadvise_result < 0 {
        return Err(map_trash_error(&path, "Unadvise", unadvise_result));
    }
    operation_result?;
    // PerformOperations has no handle-atomic postcondition. If the validated namespace path
    // still resolves, or cannot be classified unambiguously, fail closed rather than claim
    // the original target reached the Recycle Bin. A replacement can still win the residual
    // race between this check and Shell's operation; that limitation is intentional.
    verify_shell_target_absent(&path, expected_identity, &ancestor_snapshots)?;
    Ok(())
}

pub(in crate::secure_cleanup) fn shell_backend_available() -> bool {
    std::thread::spawn(|| {
        let Ok(_apartment) = ComApartment::initialize_sta() else {
            return false;
        };
        let mut file_operation = std::ptr::null_mut();
        // SAFETY: CLSID/IID are valid and output pointer is valid storage.
        let result = unsafe {
            CoCreateInstance(
                &FileOperation,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_IFILE_OPERATION,
                &mut file_operation,
            )
        };
        if result < 0 || file_operation.is_null() {
            return false;
        }
        // SAFETY: successful activation returned one owned COM interface reference.
        let _file_operation = unsafe { ComPtr::<IFileOperationTag>::from_raw(file_operation) };
        true
    })
    .join()
    .unwrap_or(false)
}

pub(super) fn map_aborted_operations(path: &Path, aborted: bool) -> CoreResult<()> {
    if aborted {
        Err(CoreError::SecureTrashAborted {
            path: path.to_path_buf(),
        })
    } else {
        Ok(())
    }
}

pub(super) fn map_trash_error(path: &Path, operation: &str, result: HRESULT) -> CoreError {
    if matches!(
        operation,
        "CoCreateInstance" | "SetOperationFlags" | "Advise"
    ) && matches!(
        result,
        -2_147_221_164 // REGDB_E_CLASSNOTREG
            | -2_147_221_163 // REGDB_E_IIDNOTREG
            | -2_147_467_263 // E_NOTIMPL
            | -2_147_467_262 // E_NOINTERFACE
            | -2_147_024_846 // HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED)
            | -2_147_024_776 // HRESULT_FROM_WIN32(ERROR_CALL_NOT_IMPLEMENTED)
    ) {
        return CoreError::SecureTrashUnavailable;
    }
    CoreError::SecureTrashOperationFailed {
        path: path.to_path_buf(),
        operation: operation.to_owned(),
        source: hresult_error(result),
    }
}

pub(super) fn revalidate_shell_target(path: &Path, expected: FileIdentity) -> CoreResult<()> {
    let entry = open_entry(path, path, AccessProfile::Inspect)?;
    if entry.identity != expected {
        return Err(CoreError::TargetIdentityChanged {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

pub(super) fn revalidate_shell_ancestor(
    ancestor: &AncestorSnapshot,
    target: &Path,
) -> CoreResult<()> {
    if target.strip_prefix(&ancestor.path).is_err() {
        return Err(CoreError::TargetIdentityChanged {
            path: target.to_path_buf(),
        });
    }
    let entry = open_directory(&ancestor.path, &ancestor.path, AccessProfile::Inspect)?;
    if entry.identity != ancestor.identity {
        return Err(CoreError::TargetIdentityChanged {
            path: ancestor.path.clone(),
        });
    }
    Ok(())
}

pub(super) fn verify_shell_target_absent(
    path: &Path,
    expected: FileIdentity,
    ancestors: &[AncestorSnapshot],
) -> CoreResult<()> {
    for ancestor in ancestors {
        revalidate_shell_ancestor(ancestor, path).map_err(|_| {
            CoreError::SecureTrashPostconditionFailed {
                path: path.to_path_buf(),
            }
        })?;
    }
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CoreError::SecureTrashPostconditionFailed {
            path: path.to_path_buf(),
        }),
        Ok(_) => match open_entry(path, path, AccessProfile::Inspect) {
            Ok(entry) if entry.identity != expected => {
                Err(CoreError::SecureTrashPostconditionFailed {
                    path: path.to_path_buf(),
                })
            }
            Ok(_) | Err(_) => Err(CoreError::SecureTrashPostconditionFailed {
                path: path.to_path_buf(),
            }),
        },
    }
}
