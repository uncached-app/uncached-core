use crate::error::{CoreError, CoreResult};
use crate::model::{FileIdentity, ScanTarget};
use std::ffi::c_void;
use std::ffi::OsString;
use std::marker::PhantomData;
use std::mem::{offset_of, size_of, MaybeUninit};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, FileIdExtdDirectoryInfo, GetFileInformationByHandleEx,
    SetFileInformationByHandle, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_TAG_INFO, FILE_DISPOSITION_FLAG_DELETE,
    FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    FILE_DISPOSITION_INFO_EX, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_FLAG_POSIX_SEMANTICS, FILE_ID_EXTD_DIR_INFO, FILE_ID_INFO, FILE_LIST_DIRECTORY,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows_sys::Win32::UI::Shell::{
    FileOperation, SHCreateItemFromParsingName, FOF_ALLOWUNDO, FOF_NO_UI,
};

const IID_ISHELL_ITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
const IID_IFILE_OPERATION: GUID = GUID::from_u128(0x947aab5f_0a5c_4c13_b4d6_4bf7836fc9f8);

#[repr(C)]
struct IUnknownVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IFileOperationVtbl {
    base: IUnknownVtbl,
    advise: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32) -> HRESULT,
    unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    set_operation_flags: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    set_progress_message: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    set_progress_dialog: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    set_properties: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    set_owner_window: unsafe extern "system" fn(*mut c_void, isize) -> HRESULT,
    apply_properties_to_item: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    apply_properties_to_items: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    rename_item:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *const u16, *mut c_void) -> HRESULT,
    rename_items: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u16) -> HRESULT,
    move_item: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const u16,
        *mut c_void,
    ) -> HRESULT,
    move_items: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    copy_item: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const u16,
        *mut c_void,
    ) -> HRESULT,
    copy_items: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    delete_item: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    delete_items: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    new_item: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *const u16,
        *const u16,
        *mut c_void,
    ) -> HRESULT,
    perform_operations: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_any_operations_aborted: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
}

#[derive(Debug)]
struct ComPtr<T> {
    ptr: *mut c_void,
    _interface: PhantomData<T>,
}

impl<T> ComPtr<T> {
    unsafe fn from_raw(ptr: *mut c_void) -> Self {
        Self {
            ptr,
            _interface: PhantomData,
        }
    }
}

impl<T> Drop for ComPtr<T> {
    fn drop(&mut self) {
        // SAFETY: COM owns pointer and its vtable remains valid until Release.
        unsafe {
            let vtable = *(self.ptr as *mut *const IUnknownVtbl);
            ((*vtable).release)(self.ptr);
        }
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize_sta() -> CoreResult<Self> {
        // SAFETY: This initializes COM only on current thread; guard uninitializes it on Drop.
        let result = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        if result < 0 {
            return Err(CoreError::SecureTrashInitializationFailed {
                source: hresult_error(result),
            });
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: this guard owns successful COM initialization on current thread.
        unsafe { CoUninitialize() };
    }
}

#[derive(Debug)]
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle owns a successful CreateFileW result.
        unsafe { CloseHandle(self.0) };
    }
}

#[derive(Debug)]
struct Entry {
    handle: Handle,
    is_directory: bool,
    identity: FileIdentity,
    path: PathBuf,
}

#[derive(Debug)]
struct Ancestor {
    handle: Handle,
    identity: FileIdentity,
    path: PathBuf,
}

#[derive(Debug)]
struct AncestorSnapshot {
    identity: FileIdentity,
    path: PathBuf,
}

#[derive(Debug)]
pub(super) struct WindowsTarget {
    _ancestors: Vec<Ancestor>,
    entries: Vec<Entry>,
}

impl WindowsTarget {
    pub(super) fn capabilities(&self) -> super::CleanupCapabilities {
        super::CleanupCapabilities {
            permanent: true,
            trash: true,
        }
    }

    pub(super) fn delete_permanently(mut self) -> CoreResult<()> {
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
        } = self;
        // open_secure_target asserts this invariant: target entry is always first and the
        // collection is never empty. An empty collection is an internal construction bug, not
        // an unavailable Trash backend.
        let target = entries
            .first()
            .expect("WindowsTarget must contain its verified target entry");
        let path = target.path.clone();
        let expected_identity = target.identity;
        let ancestor_snapshots: Vec<_> = ancestors
            .iter()
            .map(|ancestor| AncestorSnapshot {
                identity: ancestor.identity,
                path: ancestor.path.clone(),
            })
            .collect();
        // Shell operates by namespace path, so retained verification handles must be released
        // before IFileOperation. Revalidate immediately after release. This closes the known
        // handle-to-path gap, but cannot make the later Shell operation handle-atomic: the
        // approved design accepts that residual namespace race. Any identity or reparse-point
        // ambiguity detected here fails closed.
        drop(entries);
        drop(ancestors);
        for ancestor in &ancestor_snapshots {
            revalidate_shell_ancestor(ancestor, &path)?;
        }
        revalidate_shell_target(&path, expected_identity)?;
        let _apartment = ComApartment::initialize_sta()?;
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
        // SAFETY: shell_item is valid and progress sink is intentionally null.
        let result = unsafe {
            ((*vtable).delete_item)(file_operation.ptr, shell_item.ptr, std::ptr::null_mut())
        };
        if result < 0 {
            return Err(map_trash_error(&path, "DeleteItem", result));
        }
        // SAFETY: queued operation belongs to valid file_operation object.
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
        if aborted != 0 {
            return Err(CoreError::SecureTrashAborted { path });
        }
        // PerformOperations has no handle-atomic postcondition. If the validated namespace path
        // still resolves, or cannot be classified unambiguously, fail closed rather than claim
        // the original target reached the Recycle Bin. A replacement can still win the residual
        // race between this check and Shell's operation; that limitation is intentional.
        verify_shell_postcondition(&path, expected_identity)?;
        Ok(())
    }
}

// windows-sys exposes these Shell COM interfaces through their ABI, not typed wrappers.
enum IShellItemTag {}
enum IFileOperationTag {}

fn hresult_error(result: HRESULT) -> std::io::Error {
    std::io::Error::from_raw_os_error(result)
}

fn map_trash_error(path: &Path, operation: &str, result: HRESULT) -> CoreError {
    CoreError::SecureTrashOperationFailed {
        path: path.to_path_buf(),
        operation: operation.to_owned(),
        source: hresult_error(result),
    }
}

fn revalidate_shell_target(path: &Path, expected: FileIdentity) -> CoreResult<()> {
    let entry = open_entry(path, path)?;
    if entry.identity != expected {
        return Err(CoreError::TargetIdentityChanged {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn revalidate_shell_ancestor(ancestor: &AncestorSnapshot, target: &Path) -> CoreResult<()> {
    if target.strip_prefix(&ancestor.path).is_err() {
        return Err(CoreError::TargetIdentityChanged {
            path: target.to_path_buf(),
        });
    }
    let entry = open_directory(&ancestor.path, &ancestor.path)?;
    if entry.identity != ancestor.identity {
        return Err(CoreError::TargetIdentityChanged {
            path: ancestor.path.clone(),
        });
    }
    Ok(())
}

fn verify_shell_postcondition(path: &Path, _expected: FileIdentity) -> CoreResult<()> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CoreError::SecureTrashPostconditionFailed {
            path: path.to_path_buf(),
        }),
        Ok(_) => Err(CoreError::SecureTrashPostconditionFailed {
            path: path.to_path_buf(),
        }),
    }
}

pub(super) fn open_secure_target(root: &Path, target: &ScanTarget) -> CoreResult<WindowsTarget> {
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
    let root_entry = open_directory(root, &target.path)?;
    ancestors.push(Ancestor {
        handle: root_entry.handle,
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
        let entry = open_directory(&current, &target.path)?;
        ancestors.push(Ancestor {
            handle: entry.handle,
            identity: entry.identity,
            path: entry.path,
        });
    }

    let target_path = current.join(basename);
    let target_entry = open_entry(&target_path, &target.path)?;
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
    collect_entries(target_entry, &mut entries)?;
    assert!(
        !entries.is_empty(),
        "collect_entries must retain verified target entry"
    );
    Ok(WindowsTarget {
        _ancestors: ancestors,
        entries,
    })
}

fn collect_entries(entry: Entry, entries: &mut Vec<Entry>) -> CoreResult<()> {
    if entry.is_directory {
        let children =
            enumerate_directory(entry.handle.0, &entry.path, entry.identity.volume_serial)?;
        entries.push(entry);
        for child in children {
            collect_entries(child, entries)?;
        }
    } else {
        entries.push(entry);
    }
    Ok(())
}

fn open_directory(path: &Path, display: &Path) -> CoreResult<Entry> {
    let entry = open_entry(path, display)?;
    if !entry.is_directory {
        return Err(open_error_path(
            display,
            "path component is not a directory",
        ));
    }
    Ok(entry)
}

fn open_entry(path: &Path, display: &Path) -> CoreResult<Entry> {
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
    // FILE_LIST_DIRECTORY is required for directory enumeration but is not a
    // valid desired access for every regular-file provider.
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

fn open_entry_with_access(
    path: &Path,
    display: &Path,
    access: u32,
    can_enumerate: bool,
) -> CoreResult<Entry> {
    let wide = wide_path(path);
    // SAFETY: wide is NUL-terminated; all other arguments are fixed valid flags.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_POSIX_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(CoreError::SecureTargetOpenFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    open_entry_with_handle(handle, path, display, can_enumerate, None)
}

fn open_entry_with_handle(
    handle: HANDLE,
    path: &Path,
    display: &Path,
    can_enumerate: bool,
    required_access_error: Option<std::io::Error>,
) -> CoreResult<Entry> {
    let handle = Handle(handle);
    let attributes = attributes_from_handle(handle.0, display)?;
    if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || attributes.ReparseTag != 0 {
        return Err(CoreError::TargetReparsePoint {
            path: display.to_path_buf(),
        });
    }
    let is_directory = attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if is_directory && !can_enumerate {
        return Err(open_error_path_with_source(
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
            info.as_mut_ptr() as *mut c_void,
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

fn identity_from_handle(handle: HANDLE, path: &Path) -> CoreResult<FileIdentity> {
    let mut info = MaybeUninit::<FILE_ID_INFO>::zeroed();
    // SAFETY: buffer is valid writable storage of the requested class size.
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            windows_sys::Win32::Storage::FileSystem::FileIdInfo,
            info.as_mut_ptr() as *mut c_void,
            size_of::<FILE_ID_INFO>() as u32,
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

fn enumerate_directory(
    handle: HANDLE,
    display: &Path,
    volume_serial: u64,
) -> CoreResult<Vec<Entry>> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut result = Vec::new();
    loop {
        // SAFETY: buffer is writable and handle is an open directory handle.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdExtdDirectoryInfo,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as u32,
            )
        };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(18) {
                break;
            }
            return Err(CoreError::SecureEnumerationFailed {
                path: display.to_path_buf(),
                source: error,
            });
        }
        for record in parse_directory_records(&buffer).map_err(|source| {
            CoreError::SecureEnumerationFailed {
                path: display.to_path_buf(),
                source,
            }
        })? {
            let name = record.name.as_slice();
            if name != [46] && name != [46, 46] {
                let name = os_string_from_wide(name);
                let path = display.join(&name);
                let entry = open_entry(&path, &path)?;
                let enumerated = FileIdentity {
                    volume_serial,
                    file_id: record.file_id,
                };
                if !identity_matches(enumerated, entry.identity) {
                    return Err(CoreError::TargetIdentityChanged { path });
                }
                result.push(entry);
            }
        }
    }
    Ok(result)
}

#[derive(Debug)]
struct ParsedDirectoryRecord {
    name: Vec<u16>,
    file_id: [u8; 16],
}

fn parse_directory_records(buffer: &[u8]) -> Result<Vec<ParsedDirectoryRecord>, std::io::Error> {
    let header_size = offset_of!(FILE_ID_EXTD_DIR_INFO, FileName);
    let next_offset_offset = offset_of!(FILE_ID_EXTD_DIR_INFO, NextEntryOffset);
    let file_id_offset = offset_of!(FILE_ID_EXTD_DIR_INFO, FileId);
    let name_length_offset = offset_of!(FILE_ID_EXTD_DIR_INFO, FileNameLength);
    if header_size > size_of::<FILE_ID_EXTD_DIR_INFO>()
        || next_offset_offset.checked_add(size_of::<u32>()).is_none()
        || file_id_offset.checked_add(16).is_none()
        || name_length_offset.checked_add(size_of::<u32>()).is_none()
        || next_offset_offset + size_of::<u32>() > header_size
        || file_id_offset + 16 > header_size
        || name_length_offset + size_of::<u32>() > header_size
    {
        return Err(invalid_directory_record("invalid directory record layout"));
    }

    let mut records = Vec::new();
    let mut offset = 0usize;
    loop {
        if offset >= buffer.len() {
            return Err(invalid_directory_record(
                "directory record offset is out of bounds",
            ));
        }
        let remaining = buffer.len() - offset;
        if remaining < header_size {
            return Err(invalid_directory_record(
                "directory record header is truncated",
            ));
        }

        let next_offset_position = offset
            .checked_add(next_offset_offset)
            .ok_or_else(|| invalid_directory_record("directory record offset overflows"))?;
        let name_length_position = offset
            .checked_add(name_length_offset)
            .ok_or_else(|| invalid_directory_record("directory record offset overflows"))?;
        let file_id_position = offset
            .checked_add(file_id_offset)
            .ok_or_else(|| invalid_directory_record("directory record offset overflows"))?;
        let next_entry_offset = read_unaligned_at::<u32>(buffer, next_offset_position);
        let file_name_length = read_unaligned_at::<u32>(buffer, name_length_position);
        if !file_name_length.is_multiple_of(2) {
            return Err(invalid_directory_record(
                "directory record name length is odd",
            ));
        }
        let name_length = file_name_length as usize;
        let name_start = offset
            .checked_add(header_size)
            .ok_or_else(|| invalid_directory_record("directory record offset overflows"))?;
        let name_end = name_start
            .checked_add(name_length)
            .ok_or_else(|| invalid_directory_record("directory record name length overflows"))?;
        if name_end > buffer.len() {
            return Err(invalid_directory_record(
                "directory record name is out of bounds",
            ));
        }

        let record_length = if next_entry_offset == 0 {
            remaining
        } else {
            let next = next_entry_offset as usize;
            if !next.is_multiple_of(8) || next < name_end - offset || next > remaining {
                return Err(invalid_directory_record(
                    "invalid directory record next offset",
                ));
            }
            next
        };

        let file_id = read_unaligned_at::<[u8; 16]>(buffer, file_id_position);
        let name = buffer[name_start..name_end]
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        records.push(ParsedDirectoryRecord { name, file_id });

        if next_entry_offset == 0 {
            break;
        }
        offset = offset
            .checked_add(record_length)
            .ok_or_else(|| invalid_directory_record("directory record offset overflows"))?;
    }
    Ok(records)
}

fn read_unaligned_at<T: Copy>(buffer: &[u8], offset: usize) -> T {
    // SAFETY: callers validate field bounds before reading; unaligned access is intentional.
    unsafe { std::ptr::read_unaligned(buffer.as_ptr().add(offset).cast()) }
}

fn invalid_directory_record(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn map_disposition_error(path: &Path, source: std::io::Error) -> CoreError {
    match source.raw_os_error() {
        Some(code)
            if code == ERROR_INVALID_FUNCTION as i32
                || code == ERROR_NOT_SUPPORTED as i32
                || code == ERROR_INVALID_PARAMETER as i32 =>
        {
            CoreError::SecureOperationUnsupported {
                path: path.to_path_buf(),
                operation: "Windows FileDispositionInfoEx".to_owned(),
                source,
            }
        }
        _ => CoreError::SecureDeletionFailed {
            path: path.to_path_buf(),
            source,
        },
    }
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn shell_wide_path(path: &Path) -> Vec<u16> {
    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let extended_prefix = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    let unc_prefix = [b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];
    let path = if wide.starts_with(&extended_prefix) {
        if wide[4..].starts_with(&unc_prefix) {
            let mut unc = vec![b'\\' as u16, b'\\' as u16];
            unc.extend_from_slice(&wide[8..]);
            unc
        } else {
            // Only extended drive paths may discard the four-character prefix directly.
            wide[4..].to_vec()
        }
    } else {
        wide
    };
    path.into_iter().chain(std::iter::once(0)).collect()
}

fn os_string_from_wide(name: &[u16]) -> OsString {
    OsString::from_wide(name)
}

fn identity_matches(enumerated: FileIdentity, opened: FileIdentity) -> bool {
    enumerated == opened
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
mod tests {
    use super::{
        identity_matches, map_disposition_error, map_trash_error, open_entry_with_access,
        open_error_path_with_source, open_secure_target, os_string_from_wide,
        parse_directory_records, revalidate_shell_ancestor, revalidate_shell_target,
        shell_wide_path, verify_shell_postcondition, AncestorSnapshot,
    };
    use crate::error::CoreError;
    use crate::filesystem::file_identity;
    use crate::model::{Category, ScanTarget};
    use std::ffi::OsString;
    use std::fs;
    use std::mem::offset_of;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    #[test]
    fn directory_record_name_preserves_unpaired_utf16_units() {
        let name = os_string_from_wide(&[b'f' as u16, 0xd800, b'x' as u16]);
        assert_eq!(
            name,
            OsString::from_wide(&[b'f' as u16, 0xd800, b'x' as u16])
        );
    }

    #[test]
    fn shell_path_strips_extended_drive_prefix() {
        let wide = shell_wide_path(PathBuf::from(r"\\?\C:\target").as_path());

        assert_eq!(
            OsString::from_wide(&wide[..wide.len() - 1]),
            OsString::from(r"C:\target")
        );
    }

    #[test]
    fn shell_path_converts_extended_unc_prefix() {
        let wide = shell_wide_path(PathBuf::from(r"\\?\UNC\server\share\target").as_path());

        assert_eq!(
            OsString::from_wide(&wide[..wide.len() - 1]),
            OsString::from(r"\\server\share\target")
        );
    }

    #[test]
    fn shell_hresult_mapping_preserves_operation_and_hresult() {
        let path = PathBuf::from("target");
        let hresult = -2_147_467_259;
        let error = map_trash_error(&path, "PerformOperations", hresult);

        assert!(matches!(
            error,
            CoreError::SecureTrashOperationFailed { ref path, ref operation, ref source }
                if path == &PathBuf::from("target")
                    && operation == "PerformOperations"
                    && source.raw_os_error() == Some(hresult)
        ));
    }

    #[test]
    fn directory_record_identity_must_match_open_handle() {
        let expected = crate::model::FileIdentity {
            volume_serial: 7,
            file_id: [3; 16],
        };
        assert!(identity_matches(expected, expected));
        assert!(!identity_matches(
            expected,
            crate::model::FileIdentity {
                volume_serial: 7,
                file_id: [4; 16],
            }
        ));
    }

    #[test]
    fn malformed_directory_record_with_truncated_header_is_rejected() {
        let header = offset_of!(
            windows_sys::Win32::Storage::FileSystem::FILE_ID_EXTD_DIR_INFO,
            FileName
        );
        let buffer = vec![0u8; header - 1];

        assert!(parse_directory_records(&buffer).is_err());
    }

    #[test]
    fn malformed_directory_record_with_odd_name_length_is_rejected() {
        let buffer = valid_directory_record(0, 3, 0);

        assert!(parse_directory_records(&buffer).is_err());
    }

    #[test]
    fn malformed_directory_record_offsets_are_rejected() {
        let header = offset_of!(
            windows_sys::Win32::Storage::FileSystem::FILE_ID_EXTD_DIR_INFO,
            FileName
        );
        for next_offset in [header as u32, (header + 2) as u32, 1, u32::MAX] {
            let buffer = valid_directory_record(next_offset, 2, 0);
            assert!(
                parse_directory_records(&buffer).is_err(),
                "next offset {next_offset} must be rejected"
            );
        }
    }

    #[test]
    fn malformed_directory_record_name_slice_is_rejected() {
        let header = offset_of!(
            windows_sys::Win32::Storage::FileSystem::FILE_ID_EXTD_DIR_INFO,
            FileName
        );
        let buffer = valid_directory_record(0, 2, header + 1);

        assert!(parse_directory_records(&buffer).is_err());
    }

    #[test]
    fn unsupported_disposition_errors_preserve_path_and_source() {
        let path = PathBuf::from("target");
        for code in [1, 50, 87] {
            let error = map_disposition_error(&path, std::io::Error::from_raw_os_error(code));
            assert!(matches!(
                error,
                CoreError::SecureOperationUnsupported { ref path, ref source, .. }
                    if path == &PathBuf::from("target") && source.raw_os_error() == Some(code)
            ));
        }

        assert!(matches!(
            map_disposition_error(&path, std::io::Error::from_raw_os_error(5)),
            CoreError::SecureDeletionFailed { .. }
        ));
    }

    fn valid_directory_record(next_offset: u32, name_length: u32, buffer_length: usize) -> Vec<u8> {
        let header = offset_of!(
            windows_sys::Win32::Storage::FileSystem::FILE_ID_EXTD_DIR_INFO,
            FileName
        );
        let length = if buffer_length == 0 {
            header + name_length as usize
        } else {
            buffer_length
        };
        let mut buffer = vec![0u8; length];
        write_unaligned(&mut buffer, 0, next_offset);
        write_unaligned(
            &mut buffer,
            offset_of!(
                windows_sys::Win32::Storage::FileSystem::FILE_ID_EXTD_DIR_INFO,
                FileNameLength
            ),
            name_length,
        );
        buffer
    }

    fn write_unaligned<T: Copy>(buffer: &mut [u8], offset: usize, value: T) {
        assert!(offset + std::mem::size_of::<T>() <= buffer.len());
        // SAFETY: bounds are checked and the destination may be unaligned.
        unsafe { std::ptr::write_unaligned(buffer.as_mut_ptr().add(offset).cast(), value) };
    }

    #[test]
    fn directory_opened_without_enumeration_access_is_rejected() {
        let fixture = Fixture::new("directory-access");
        let directory = fixture.root.join("target");
        fs::create_dir(&directory).unwrap();

        assert!(matches!(
            open_entry_with_access(
                &directory,
                &directory,
                windows_sys::Win32::Storage::FileSystem::DELETE
                    | windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES,
                false,
            ),
            Err(CoreError::SecureTargetOpenFailed { .. })
        ));
    }

    #[test]
    fn directory_access_rejection_preserves_required_open_error() {
        let path = PathBuf::from("target");
        let error = open_error_path_with_source(
            &path,
            "directory handle lacks enumeration access",
            Some(std::io::Error::from_raw_os_error(5)),
        );

        match error {
            CoreError::SecureTargetOpenFailed { source, .. } => {
                assert_eq!(source.raw_os_error(), Some(5));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn secure_open_retains_stable_identity_and_enables_cleanup() {
        let fixture = Fixture::new("stable");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("child.txt"), b"data").unwrap();
        let target = ScanTarget::new(target_path, Category::NodeModules, 0)
            .with_identity(file_identity(&fixture.root.join("target")));
        let secure = open_secure_target(&fixture.root, &target).unwrap();
        assert!(secure.capabilities().permanent);
        assert!(secure.capabilities().trash);
    }

    #[test]
    fn trash_moves_verified_target_without_mutating_contents_first() {
        let fixture = Fixture::new("unsupported-trash");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("child.txt"), b"data").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&target_path));
        let secure = open_secure_target(&fixture.root, &target).unwrap();

        assert!(secure.capabilities().trash);
        secure
            .move_to_trash()
            .expect("verified target should move to Recycle Bin");
        assert!(!target_path.exists());
    }

    #[test]
    fn secure_open_rejects_identity_mismatch_and_replacement() {
        let fixture = Fixture::new("identity");
        let original = fixture.root.join("target");
        fs::create_dir(&original).unwrap();
        let target = ScanTarget::new(original.clone(), Category::NodeModules, 0).with_identity(
            Some(crate::model::FileIdentity {
                volume_serial: 0,
                file_id: [0; 16],
            }),
        );
        assert!(matches!(
            open_secure_target(&fixture.root, &target),
            Err(CoreError::TargetIdentityChanged { .. })
        ));
        let identity = file_identity(&original);
        fs::rename(&original, fixture.root.join("old")).unwrap();
        fs::create_dir(&original).unwrap();
        let replaced = ScanTarget::new(original, Category::NodeModules, 0).with_identity(identity);
        assert!(matches!(
            open_secure_target(&fixture.root, &replaced),
            Err(CoreError::TargetIdentityChanged { .. })
        ));
    }

    #[test]
    fn secure_open_rejects_direct_and_ancestor_reparse_points() {
        let fixture = Fixture::new("reparse");
        fs::create_dir(fixture.root.join("real")).unwrap();
        if let Err(error) = std::os::windows::fs::symlink_dir("real", fixture.root.join("direct")) {
            if error.raw_os_error() == Some(1314) {
                return;
            }
            panic!("creating direct reparse fixture failed: {error}");
        }
        let direct = ScanTarget::new(fixture.root.join("direct"), Category::NodeModules, 0)
            .with_identity(file_identity(&fixture.root.join("real")));
        assert!(matches!(
            open_secure_target(&fixture.root, &direct),
            Err(CoreError::TargetReparsePoint { .. })
        ));
        fs::create_dir(fixture.root.join("real").join("target")).unwrap();
        if let Err(error) = std::os::windows::fs::symlink_dir("real", fixture.root.join("ancestor"))
        {
            if error.raw_os_error() == Some(1314) {
                return;
            }
            panic!("creating ancestor reparse fixture failed: {error}");
        }
        let ancestor = ScanTarget::new(
            fixture.root.join("ancestor").join("target"),
            Category::NodeModules,
            0,
        )
        .with_identity(file_identity(&fixture.root.join("real").join("target")));
        assert!(matches!(
            open_secure_target(&fixture.root, &ancestor),
            Err(CoreError::TargetReparsePoint { .. })
        ));
    }

    #[test]
    fn shell_revalidation_rejects_replaced_ancestor_reparse_point() {
        let fixture = Fixture::new("revalidate-ancestor");
        let real = fixture.root.join("real");
        let ancestor = fixture.root.join("ancestor");
        let target = ancestor.join("target");
        fs::create_dir(&real).unwrap();
        fs::create_dir(&ancestor).unwrap();
        fs::create_dir(&target).unwrap();

        let opened = super::open_directory(&ancestor, &target).unwrap();
        let snapshot = AncestorSnapshot {
            identity: opened.identity,
            path: opened.path.clone(),
        };
        drop(opened);
        fs::remove_dir(&target).unwrap();
        fs::remove_dir(&ancestor).unwrap();
        if let Err(error) = std::os::windows::fs::symlink_dir(&real, &ancestor) {
            if error.raw_os_error() == Some(1314) {
                return;
            }
            panic!("creating replacement ancestor junction failed: {error}");
        }

        assert!(matches!(
            revalidate_shell_ancestor(&snapshot, &target),
            Err(CoreError::TargetReparsePoint { path }) if path == ancestor
        ));
    }

    #[test]
    fn shell_revalidation_rejects_replaced_target_identity() {
        let fixture = Fixture::new("revalidate-target");
        let target = fixture.root.join("target");
        fs::create_dir(&target).unwrap();
        let expected = file_identity(&target).unwrap();
        fs::rename(&target, fixture.root.join("original")).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("replacement.txt"), b"replacement").unwrap();

        assert!(matches!(
            revalidate_shell_target(&target, expected),
            Err(CoreError::TargetIdentityChanged { path }) if path == target
        ));
        assert_eq!(
            fs::read(target.join("replacement.txt")).unwrap(),
            b"replacement"
        );
        assert!(fixture.root.join("original").exists());
    }

    #[test]
    fn shell_postcondition_rejects_replacement_as_ambiguous() {
        let fixture = Fixture::new("postcondition-replacement");
        let target = fixture.root.join("target");
        fs::create_dir(&target).unwrap();
        let expected = file_identity(&target).unwrap();
        fs::rename(&target, fixture.root.join("original")).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("replacement.txt"), b"replacement").unwrap();

        assert!(matches!(
            verify_shell_postcondition(&target, expected),
            Err(CoreError::SecureTrashPostconditionFailed { path }) if path == target
        ));
        assert_eq!(
            fs::read(target.join("replacement.txt")).unwrap(),
            b"replacement"
        );
        assert!(fixture.root.join("original").exists());
    }

    #[test]
    fn shell_postcondition_accepts_only_absent_original_path() {
        let fixture = Fixture::new("postcondition-absent");
        let target = fixture.root.join("target");
        fs::create_dir(&target).unwrap();
        let expected = file_identity(&target).unwrap();
        fs::remove_dir(&target).unwrap();

        assert!(verify_shell_postcondition(&target, expected).is_ok());
    }

    #[test]
    fn permanent_cleanup_removes_verified_tree_through_retained_handles() {
        let fixture = Fixture::new("handle-delete");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("original.txt"), b"original").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&target_path));
        let secure = open_secure_target(&fixture.root, &target).unwrap();
        secure.delete_permanently().unwrap();
        assert!(!target_path.exists());
    }

    #[test]
    fn replacement_created_after_open_handles_are_released_survives_cleanup() {
        let fixture = Fixture::new("replacement-after-release");
        let target_path = fixture.root.join("target");
        let archived_path = fixture.root.join("archived");
        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("original.txt"), b"original-content").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&target_path));
        let secure = open_secure_target(&fixture.root, &target).unwrap();

        let replacement_attempt = fs::rename(&target_path, &archived_path).unwrap_err();
        assert_eq!(replacement_attempt.raw_os_error(), Some(5));
        assert!(!archived_path.exists());

        secure.delete_permanently().unwrap();
        assert!(!target_path.exists());

        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("replacement.txt"), b"replacement-content").unwrap();
        assert_eq!(
            fs::read(target_path.join("replacement.txt")).unwrap(),
            b"replacement-content"
        );
        assert!(!target_path.join("original.txt").exists());
        assert!(!archived_path.exists());
    }

    #[test]
    fn unsupported_capability_does_not_mutate_filesystem() {
        let secure = super::super::SecureTarget {
            backend: super::super::BackendTarget::Unsupported,
        };
        assert!(!secure.capabilities().permanent);
        assert!(!secure.capabilities().trash);
        assert!(matches!(
            secure.delete_permanently(),
            Err(CoreError::SecureOperationUnsupported { .. })
        ));
    }

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "uncached-core-windows-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir(&root).unwrap();
            Self { root }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
