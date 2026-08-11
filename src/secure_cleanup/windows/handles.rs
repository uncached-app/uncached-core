use crate::error::{CoreError, CoreResult};
use crate::model::FileIdentity;
use std::mem::{size_of, MaybeUninit};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, FileIdInfo, GetFileInformationByHandleEx, DELETE,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_POSIX_SEMANTICS,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};

#[derive(Debug)]
pub(super) struct Handle(pub(super) HANDLE);

// SAFETY: a file `HANDLE` is a process-wide kernel value with no thread
// affinity, and `Handle` owns it exclusively, so moving one between threads
// cannot alias or invalidate it. Only COM interfaces are apartment-bound, and
// none are represented by this type.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle owns a successful CreateFileW result.
        unsafe { CloseHandle(self.0) };
    }
}

#[derive(Debug)]
pub(super) struct Entry {
    pub(super) handle: Handle,
    pub(super) is_directory: bool,
    pub(super) identity: FileIdentity,
    pub(super) path: PathBuf,
}

/// How much access a secure open needs from an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AccessProfile {
    Delete,
    Inspect,
}

pub(super) fn open_directory(
    path: &Path,
    display: &Path,
    access: AccessProfile,
) -> CoreResult<Entry> {
    let entry = open_entry(path, display, access)?;
    if !entry.is_directory {
        return Err(super::open_error_path(
            display,
            "path component is not a directory",
        ));
    }
    Ok(entry)
}

pub(super) fn open_entry(path: &Path, display: &Path, access: AccessProfile) -> CoreResult<Entry> {
    let wide = wide_path(path);
    let open = |access| {
        // SAFETY: wide is NUL-terminated; all other arguments are fixed valid flags.
        unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_POSIX_SEMANTICS,
                std::ptr::null_mut(),
            )
        }
    };

    if access == AccessProfile::Inspect {
        let handle = open(FILE_READ_ATTRIBUTES);
        if handle == INVALID_HANDLE_VALUE {
            return Err(CoreError::SecureTargetOpenFailed {
                path: display.to_path_buf(),
                source: std::io::Error::last_os_error(),
            });
        }
        return open_entry_with_handle(handle, path, display, true, None);
    }

    let handle = open(DELETE | FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY);
    if handle != INVALID_HANDLE_VALUE {
        return open_entry_with_handle(handle, path, display, true, None);
    }
    let required_access_error = std::io::Error::last_os_error();
    let handle = open(DELETE | FILE_READ_ATTRIBUTES);
    if handle != INVALID_HANDLE_VALUE {
        return open_entry_with_handle(handle, path, display, false, Some(required_access_error));
    }
    Err(CoreError::SecureTargetOpenFailed {
        path: display.to_path_buf(),
        source: std::io::Error::last_os_error(),
    })
}

pub(super) fn open_entry_with_handle(
    raw_handle: HANDLE,
    path: &Path,
    display: &Path,
    can_enumerate: bool,
    required_access_error: Option<std::io::Error>,
) -> CoreResult<Entry> {
    let handle = Handle(raw_handle);
    let attributes = attributes_from_handle(handle.0, display)?;
    if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || attributes.ReparseTag != 0 {
        return Err(CoreError::TargetReparsePoint {
            path: display.to_path_buf(),
        });
    }
    let is_directory = attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if is_directory && !can_enumerate {
        return Err(super::open_error_path_with_source(
            display,
            "directory handle lacks enumeration access",
            required_access_error,
        ));
    }
    let identity = identity_from_handle(handle.0, display)?;
    Ok(Entry {
        is_directory,
        identity,
        handle,
        path: path.to_path_buf(),
    })
}

fn attributes_from_handle(handle: HANDLE, path: &Path) -> CoreResult<FILE_ATTRIBUTE_TAG_INFO> {
    let mut info = MaybeUninit::<FILE_ATTRIBUTE_TAG_INFO>::zeroed();
    // SAFETY: buffer is valid writable storage of the requested class size.
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            info.as_mut_ptr() as *mut _,
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(CoreError::SecureHandleQueryFailed {
            path: path.to_path_buf(),
            operation: "attributes",
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: successful API call initialized buffer.
    Ok(unsafe { info.assume_init() })
}

pub(super) fn identity_from_handle(handle: HANDLE, path: &Path) -> CoreResult<FileIdentity> {
    let mut info = MaybeUninit::<windows_sys::Win32::Storage::FileSystem::FILE_ID_INFO>::zeroed();
    // SAFETY: buffer is valid writable storage of the requested class size.
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            info.as_mut_ptr() as *mut _,
            size_of::<windows_sys::Win32::Storage::FileSystem::FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(CoreError::SecureHandleQueryFailed {
            path: path.to_path_buf(),
            operation: "identity",
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: successful API call initialized buffer.
    let info = unsafe { info.assume_init() };
    Ok(FileIdentity {
        volume_serial: info.VolumeSerialNumber,
        file_id: info.FileId.Identifier,
    })
}

pub(super) fn identity_matches(enumerated: FileIdentity, opened: FileIdentity) -> bool {
    enumerated == opened
}

pub(super) fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
