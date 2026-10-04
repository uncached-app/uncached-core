use super::CleanupIntent;
mod directory_parser;
mod handles;
use crate::error::{CoreError, CoreResult};
use crate::model::{FileIdentity, ScanTarget};
use directory_parser::{os_string_from_wide, parse_directory_records};
use handles::{
    identity_from_handle, identity_matches, open_directory, open_entry, AccessProfile, Entry,
    Handle,
};
#[cfg(test)]
use handles::{open_entry_with_handle, wide_path};
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use std::cell::RefCell;
use std::ffi::c_void;
use std::ffi::OsString;
use std::marker::PhantomData;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{
    ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, HANDLE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdExtdDirectoryInfo, GetFileInformationByHandleEx, SetFileInformationByHandle,
    FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO_EX,
};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows_sys::Win32::UI::Shell::{
    FileOperation, SHCreateItemFromParsingName, FOF_ALLOWUNDO, FOF_NO_UI,
};

mod com;
mod enumerate;
mod progress_sink;
mod trash;

use com::*;
use enumerate::*;
use progress_sink::*;
pub(super) use trash::shell_backend_available;
use trash::*;
#[cfg(test)]
pub(crate) use trash::{force_next_trash_abort, install_shell_item_boundary_hook};

#[derive(Debug)]
pub(super) struct WindowsTarget {
    _ancestors: Vec<Ancestor>,
    entries: Vec<Entry>,
    intent: CleanupIntent,
}

impl WindowsTarget {
    pub(super) fn delete_permanently(mut self) -> CoreResult<()> {
        if self.intent != CleanupIntent::Permanent {
            // A Trash-opened target holds no descendant handles, so it cannot
            // delete a tree. Refuse instead of half-deleting one.
            return Err(CoreError::SecureOperationUnsupported {
                path: self
                    .entries
                    .first()
                    .map(|entry| entry.path.clone())
                    .unwrap_or_default(),
                operation: "permanent cleanup".to_owned(),
                source: std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "secure target was opened for Trash cleanup",
                ),
            });
        }
        for entry in self.entries.drain(..).rev() {
            let info = FILE_DISPOSITION_INFO_EX {
                Flags: FILE_DISPOSITION_FLAG_DELETE
                    | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
                    | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
            };
            // SAFETY: entry.handle is valid, and info is initialized for this class.
            let ok = unsafe {
                SetFileInformationByHandle(
                    entry.handle.0,
                    windows_sys::Win32::Storage::FileSystem::FileDispositionInfoEx,
                    &info as *const _ as *const c_void,
                    size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
                )
            };
            if ok == 0 {
                return Err(map_disposition_error(
                    &entry.path,
                    std::io::Error::last_os_error(),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn move_to_trash(self) -> CoreResult<()> {
        let WindowsTarget {
            entries,
            _ancestors: ancestors,
            ..
        } = self;
        // open_secure_target asserts this invariant: target entry is always first and the
        // collection is never empty. An empty collection is an internal construction bug, not
        // an unavailable Trash backend.
        let target = entries
            .first()
            .ok_or_else(|| CoreError::SecureTrashInitializationFailed {
                source: std::io::Error::other(
                    "secure target did not retain its verified target entry",
                ),
            })?;
        let path = target.path.clone();
        let expected_identity = target.identity;
        let ancestor_snapshots: Vec<_> = ancestors
            .iter()
            .map(|ancestor| AncestorSnapshot {
                identity: ancestor.identity,
                path: ancestor.path.clone(),
            })
            .collect();
        drop(entries);
        drop(ancestors);
        let input = ShellTrashInput {
            path,
            expected_identity,
            ancestor_snapshots,
            #[cfg(test)]
            boundary_hook: take_shell_item_boundary_hook(),
            #[cfg(test)]
            force_aborted_result: take_forced_trash_abort(),
        };
        std::thread::spawn(move || run_trash_on_sta(input))
            .join()
            .map_err(|_| CoreError::SecureTrashInitializationFailed {
                source: std::io::Error::other("Windows Shell STA thread panicked"),
            })?
    }
}

pub(super) fn open_secure_target(
    root: &Path,
    target: &ScanTarget,
    intent: CleanupIntent,
) -> CoreResult<WindowsTarget> {
    let expected = target
        .identity
        .ok_or_else(|| CoreError::TargetIdentityUnavailable {
            path: target.path.clone(),
        })?;
    let relative = target
        .path
        .strip_prefix(root)
        .map_err(|_| open_error(target, "target is outside workspace"))?;
    let components: Vec<_> = relative.components().collect();
    let Some(Component::Normal(basename)) = components.last() else {
        return Err(open_error(target, "target must have a normal basename"));
    };

    let mut ancestors = Vec::new();
    let mut current = root.to_path_buf();
    let root_entry = open_directory(root, root, AccessProfile::Inspect)?;
    ancestors.push(Ancestor {
        _handle: root_entry.handle,
        identity: root_entry.identity,
        path: root_entry.path,
    });
    for component in &components[..components.len() - 1] {
        let Component::Normal(name) = component else {
            return Err(open_error(
                target,
                "target path contains unsupported component",
            ));
        };
        current.push(name);
        let entry = open_directory(&current, &current, AccessProfile::Inspect)?;
        ancestors.push(Ancestor {
            _handle: entry.handle,
            identity: entry.identity,
            path: entry.path,
        });
    }

    let target_path = current.join(basename);
    let target_access = match intent {
        CleanupIntent::Permanent => AccessProfile::Delete,
        CleanupIntent::Trash => AccessProfile::Inspect,
    };
    let target_entry = open_entry(&target_path, &target.path, target_access)?;
    if !target_entry.is_directory {
        return Err(open_error(
            target,
            "secure cleanup target is not a directory",
        ));
    }
    if identity_from_handle(target_entry.handle.0, &target.path)? != expected {
        return Err(CoreError::TargetIdentityChanged {
            path: target.path.clone(),
        });
    }

    let mut entries = Vec::new();
    match intent {
        // Permanent cleanup deletes through the retained handles, so every entry
        // has to be opened and verified before anything is mutated.
        CleanupIntent::Permanent => collect_entries(target_entry, &mut entries)?,
        // Trash cleanup hands a path to the Shell and releases every handle
        // first, so descendant handles would be opened, held and dropped without
        // ever guarding the operation. Only the target and its ancestors are
        // pinned; the Shell moves the target as a single item and does not
        // traverse reparse points inside it.
        CleanupIntent::Trash => entries.push(target_entry),
    }
    if entries.is_empty() {
        return Err(CoreError::SecureTargetOpenFailed {
            path: target.path.clone(),
            source: std::io::Error::other("secure open did not retain the verified target entry"),
        });
    }
    Ok(WindowsTarget {
        _ancestors: ancestors,
        entries,
        intent,
    })
}

fn open_error(target: &ScanTarget, message: &str) -> CoreError {
    open_error_path(&target.path, message)
}

fn open_error_path(path: &Path, message: &str) -> CoreError {
    open_error_path_with_source(path, message, None)
}

fn open_error_path_with_source(
    path: &Path,
    message: &str,
    source: Option<std::io::Error>,
) -> CoreError {
    CoreError::SecureTargetOpenFailed {
        path: path.to_path_buf(),
        source: source
            .unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, message)),
    }
}

#[cfg(test)]
mod tests;
