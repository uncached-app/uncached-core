use crate::error::{CoreError, CoreResult};
use crate::model::{FileIdentity, ScanTarget};
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use std::cell::RefCell;
use std::ffi::c_void;
use std::ffi::OsString;
use std::marker::PhantomData;
use std::mem::{offset_of, size_of, MaybeUninit};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
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
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows_sys::Win32::UI::Shell::{
    FileOperation, SHCreateItemFromParsingName, FOF_ALLOWUNDO, FOF_NO_UI,
};

const IID_ISHELL_ITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
const IID_IFILE_OPERATION: GUID = GUID::from_u128(0x947aab5f_0a5c_4c13_b4d6_4bf7836fc9f8);
const IID_IFILE_OPERATION_PROGRESS_SINK: GUID =
    GUID::from_u128(0x04b0f1a7_9490_44bc_96e1_4296a31252e2);
const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);

#[cfg(test)]
type ShellItemBoundaryHook = Box<dyn FnOnce() + Send>;

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
fn take_shell_item_boundary_hook() -> Option<ShellItemBoundaryHook> {
    SHELL_ITEM_BOUNDARY_HOOK.with(|slot| slot.borrow_mut().take())
}

#[cfg(test)]
pub(crate) fn force_next_trash_abort() {
    FORCE_TRASH_ABORT.with(|abort| abort.set(true));
}

#[cfg(test)]
fn take_forced_trash_abort() -> bool {
    FORCE_TRASH_ABORT.with(|abort| abort.replace(false))
}

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

#[repr(C)]
struct IFileOperationProgressSinkVtbl {
    base: IUnknownVtbl,
    start_operations: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    finish_operations: unsafe extern "system" fn(*mut c_void, HRESULT) -> HRESULT,
    pre_rename_item:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *const u16) -> HRESULT,
    post_rename_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *const u16,
        HRESULT,
        *mut c_void,
    ) -> HRESULT,
    pre_move_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *const u16,
    ) -> HRESULT,
    post_move_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *const u16,
        HRESULT,
        *mut c_void,
    ) -> HRESULT,
    pre_copy_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *const u16,
    ) -> HRESULT,
    post_copy_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *const u16,
        HRESULT,
        *mut c_void,
    ) -> HRESULT,
    pre_delete_item: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT,
    post_delete_item:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, HRESULT, *mut c_void) -> HRESULT,
    pre_new_item: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *const u16) -> HRESULT,
    post_new_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *const u16,
        *const u16,
        u32,
        HRESULT,
        *mut c_void,
    ) -> HRESULT,
    update_progress: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
    reset_timer: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pause_timer: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    resume_timer: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

#[derive(Debug, Default)]
struct TrashProgressEvidence {
    post_delete_hresult: Option<HRESULT>,
    expected_target_callback: bool,
}

fn classify_trash_progress(path: &Path, evidence: &TrashProgressEvidence) -> CoreResult<()> {
    if evidence
        .post_delete_hresult
        .is_some_and(|result| result >= 0)
        && evidence.expected_target_callback
    {
        Ok(())
    } else {
        Err(CoreError::SecureTrashPostconditionFailed {
            path: path.to_path_buf(),
        })
    }
}

#[repr(C)]
struct ProgressSink {
    vtable: *const IFileOperationProgressSinkVtbl,
    references: std::sync::atomic::AtomicU32,
    expected_path: PathBuf,
    expected_item: *mut c_void,
    evidence: Arc<Mutex<TrashProgressEvidence>>,
}

unsafe impl Send for ProgressSink {}
unsafe impl Sync for ProgressSink {}

static PROGRESS_SINK_VTABLE: IFileOperationProgressSinkVtbl = IFileOperationProgressSinkVtbl {
    base: IUnknownVtbl {
        query_interface: progress_sink_query_interface,
        add_ref: progress_sink_add_ref,
        release: progress_sink_release,
    },
    start_operations: progress_sink_ok,
    finish_operations: progress_sink_finish,
    pre_rename_item: progress_sink_pre_rename,
    post_rename_item: progress_sink_ok_post_rename,
    pre_move_item: progress_sink_pre_move,
    post_move_item: progress_sink_ok_post_move,
    pre_copy_item: progress_sink_pre_move,
    post_copy_item: progress_sink_ok_post_move,
    pre_delete_item: progress_sink_pre_delete,
    post_delete_item: progress_sink_post_delete,
    pre_new_item: progress_sink_pre_new,
    post_new_item: progress_sink_ok_post_new,
    update_progress: progress_sink_update,
    reset_timer: progress_sink_ok,
    pause_timer: progress_sink_ok,
    resume_timer: progress_sink_ok,
};

impl ProgressSink {
    fn allocate(
        expected_path: &Path,
        expected_item: *mut c_void,
    ) -> (*mut c_void, Arc<Mutex<TrashProgressEvidence>>) {
        let evidence = Arc::new(Mutex::new(TrashProgressEvidence::default()));
        let sink = Box::new(Self {
            vtable: &PROGRESS_SINK_VTABLE,
            references: std::sync::atomic::AtomicU32::new(1),
            expected_path: expected_path.to_path_buf(),
            expected_item,
            evidence: Arc::clone(&evidence),
        });
        (Box::into_raw(sink).cast(), evidence)
    }
}

unsafe fn progress_sink_from_raw<'a>(raw: *mut c_void) -> &'a ProgressSink {
    // SAFETY: COM calls provide the interface pointer originally returned by ProgressSink::new.
    unsafe { &*(raw.cast::<ProgressSink>()) }
}

unsafe extern "system" fn progress_sink_query_interface(
    raw: *mut c_void,
    iid: *const GUID,
    result: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || result.is_null() {
        return -2147467261;
    }
    // SAFETY: COM guarantees iid/result point to writable ABI values for this call.
    let iid = unsafe { &*iid };
    if guid_matches(iid, &IID_IUNKNOWN) || guid_matches(iid, &IID_IFILE_OPERATION_PROGRESS_SINK) {
        // SAFETY: result points to caller-owned output storage and raw is a valid sink.
        unsafe {
            *result = raw;
            progress_sink_add_ref(raw);
        }
        0
    } else {
        // SAFETY: result is non-null as checked above.
        unsafe { *result = std::ptr::null_mut() };
        -2147467262
    }
}

unsafe fn guid_matches(left: *const GUID, right: &GUID) -> bool {
    // SAFETY: caller validates left is non-null before comparing COM identifiers.
    unsafe { *left }.data1 == right.data1
        && unsafe { *left }.data2 == right.data2
        && unsafe { *left }.data3 == right.data3
        && unsafe { *left }.data4 == right.data4
}

unsafe extern "system" fn progress_sink_add_ref(raw: *mut c_void) -> u32 {
    // SAFETY: raw is a live COM sink while AddRef is callable.
    unsafe { progress_sink_from_raw(raw) }
        .references
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        + 1
}

unsafe extern "system" fn progress_sink_release(raw: *mut c_void) -> u32 {
    // SAFETY: raw is a live COM sink while Release is callable.
    let remaining = unsafe { progress_sink_from_raw(raw) }
        .references
        .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
        - 1;
    if remaining == 0 {
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        // SAFETY: this was the final COM reference created by Box::into_raw.
        unsafe { drop(Box::from_raw(raw.cast::<ProgressSink>())) };
    }
    remaining
}

unsafe extern "system" fn progress_sink_ok(_raw: *mut c_void) -> HRESULT {
    0
}

unsafe extern "system" fn progress_sink_finish(_raw: *mut c_void, _result: HRESULT) -> HRESULT {
    0
}

unsafe extern "system" fn progress_sink_pre_rename(
    _raw: *mut c_void,
    _flags: u32,
    _item: *mut c_void,
    _name: *const u16,
) -> HRESULT {
    0
}

unsafe extern "system" fn progress_sink_ok_post_rename(
    _raw: *mut c_void,
    _flags: u32,
    _item: *mut c_void,
    _name: *const u16,
    _result: HRESULT,
    _created: *mut c_void,
) -> HRESULT {
    0
}

unsafe extern "system" fn progress_sink_pre_move(
    _raw: *mut c_void,
    _flags: u32,
    _item: *mut c_void,
    _folder: *mut c_void,
    _name: *const u16,
) -> HRESULT {
    0
}

unsafe extern "system" fn progress_sink_pre_new(
    _raw: *mut c_void,
    _flags: u32,
    _folder: *mut c_void,
    _name: *const u16,
) -> HRESULT {
    0
}

unsafe extern "system" fn progress_sink_ok_post_move(
    _raw: *mut c_void,
    _flags: u32,
    _item: *mut c_void,
    _folder: *mut c_void,
    _name: *const u16,
    _result: HRESULT,
    _created: *mut c_void,
) -> HRESULT {
    0
}

unsafe extern "system" fn progress_sink_pre_delete(
    _raw: *mut c_void,
    _flags: u32,
    _item: *mut c_void,
) -> HRESULT {
    0
}

unsafe extern "system" fn progress_sink_post_delete(
    raw: *mut c_void,
    _flags: u32,
    item: *mut c_void,
    result: HRESULT,
    _created: *mut c_void,
) -> HRESULT {
    // SAFETY: raw is the sink instance supplied by COM and item is an optional IShellItem.
    let sink = unsafe { progress_sink_from_raw(raw) };
    let expected = item == sink.expected_item
        || unsafe { shell_item_path(item) }
            .is_some_and(|callback| shell_paths_equal(&callback, &sink.expected_path));
    if let Ok(mut evidence) = sink.evidence.lock() {
        if expected {
            evidence.post_delete_hresult = Some(result);
            evidence.expected_target_callback = true;
        } else if evidence.post_delete_hresult.is_none() {
            evidence.post_delete_hresult = Some(result);
        }
    }
    0
}

fn shell_paths_equal(left: &Path, right: &Path) -> bool {
    shell_wide_path(left) == shell_wide_path(right)
}

unsafe fn shell_item_path(item: *mut c_void) -> Option<PathBuf> {
    if item.is_null() {
        return None;
    }
    let vtable = unsafe { *(item as *mut *const ShellItemVtbl) };
    let mut display_name = std::ptr::null_mut();
    let result = unsafe { ((*vtable).get_display_name)(item, 0x8005_8000, &mut display_name) };
    if result < 0 || display_name.is_null() {
        return None;
    }
    let mut length = 0;
    while unsafe { *display_name.add(length) } != 0 {
        length += 1;
    }
    let path = PathBuf::from(OsString::from_wide(unsafe {
        std::slice::from_raw_parts(display_name, length)
    }));
    unsafe { CoTaskMemFree(display_name.cast()) };
    Some(path)
}

#[repr(C)]
struct ShellItemVtbl {
    base: IUnknownVtbl,
    bind_to_handler: *const c_void,
    get_parent: *const c_void,
    get_display_name: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> HRESULT,
}

unsafe extern "system" fn progress_sink_ok_post_new(
    _raw: *mut c_void,
    _flags: u32,
    _folder: *mut c_void,
    _name: *const u16,
    _template: *const u16,
    _attributes: u32,
    _result: HRESULT,
    _item: *mut c_void,
) -> HRESULT {
    0
}

unsafe extern "system" fn progress_sink_update(
    _raw: *mut c_void,
    _total: u32,
    _done: u32,
) -> HRESULT {
    0
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

struct ShellTrashInput {
    path: PathBuf,
    expected_identity: FileIdentity,
    ancestor_snapshots: Vec<AncestorSnapshot>,
    #[cfg(test)]
    boundary_hook: Option<ShellItemBoundaryHook>,
    #[cfg(test)]
    force_aborted_result: bool,
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
            trash: shell_backend_available(),
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

fn run_trash_on_sta(input: ShellTrashInput) -> CoreResult<()> {
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

// windows-sys exposes these Shell COM interfaces through their ABI, not typed wrappers.
enum IShellItemTag {}
enum IFileOperationTag {}

fn hresult_error(result: HRESULT) -> std::io::Error {
    std::io::Error::from_raw_os_error(result)
}

pub(super) fn shell_backend_available() -> bool {
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

fn map_aborted_operations(path: &Path, aborted: bool) -> CoreResult<()> {
    if aborted {
        Err(CoreError::SecureTrashAborted {
            path: path.to_path_buf(),
        })
    } else {
        Ok(())
    }
}

fn map_trash_error(path: &Path, operation: &str, result: HRESULT) -> CoreError {
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
            | -2_147_024_894 // HRESULT_FROM_WIN32(ERROR_CALL_NOT_IMPLEMENTED)
    ) {
        return CoreError::SecureTrashUnavailable;
    }
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

fn verify_shell_target_absent(
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
        Ok(_) => match open_entry(path, path) {
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
        classify_trash_progress, identity_matches, map_aborted_operations, map_disposition_error,
        map_trash_error, open_entry_with_access, open_error_path_with_source, open_secure_target,
        os_string_from_wide, parse_directory_records, progress_sink_add_ref, progress_sink_release,
        revalidate_shell_ancestor, revalidate_shell_target, shell_wide_path,
        verify_shell_target_absent, AncestorSnapshot, ProgressSink, TrashProgressEvidence,
    };
    use crate::error::CoreError;
    use crate::filesystem::file_identity;
    use crate::model::{Category, ScanTarget};
    use std::ffi::OsString;
    use std::fs;
    use std::mem::offset_of;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    #[test]
    fn progress_sink_release_handles_concurrent_final_releases() {
        for _ in 0..256 {
            let target = PathBuf::from("target");
            let (sink, _) = ProgressSink::allocate(&target, std::ptr::null_mut());
            // SAFETY: sink is live and owns its construction reference.
            unsafe { progress_sink_add_ref(sink) };
            let barrier = Arc::new(Barrier::new(3));
            let first_barrier = Arc::clone(&barrier);
            let second_barrier = Arc::clone(&barrier);
            let sink_address = sink as usize;
            let first = std::thread::spawn(move || {
                first_barrier.wait();
                // SAFETY: this thread releases one of two live COM references.
                unsafe { progress_sink_release(sink_address as *mut std::ffi::c_void) }
            });
            let second = std::thread::spawn(move || {
                second_barrier.wait();
                // SAFETY: this thread releases one of two live COM references.
                unsafe { progress_sink_release(sink_address as *mut std::ffi::c_void) }
            });
            barrier.wait();

            let results = [first.join().unwrap(), second.join().unwrap()];
            assert!(results.contains(&0));
            assert!(results.contains(&1));
        }
    }

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
    fn documented_shell_unavailability_maps_to_typed_unavailable_error() {
        let path = PathBuf::from("target");
        for hresult in [
            -2_147_221_164, // REGDB_E_CLASSNOTREG
            -2_147_467_262, // E_NOINTERFACE
            -2_147_024_846, // HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED)
        ] {
            assert!(matches!(
                map_trash_error(&path, "CoCreateInstance", hresult),
                CoreError::SecureTrashUnavailable
            ));
        }
    }

    #[test]
    fn get_any_operations_aborted_maps_to_typed_abort() {
        let path = PathBuf::from("target");
        assert!(matches!(
            map_aborted_operations(&path, true),
            Err(CoreError::SecureTrashAborted { path: actual }) if actual == path
        ));
        assert!(map_aborted_operations(&path, false).is_ok());
    }

    #[test]
    fn trash_requires_successful_callback_for_expected_target() {
        let path = PathBuf::from("target");
        let evidence = TrashProgressEvidence {
            post_delete_hresult: Some(0),
            expected_target_callback: true,
        };

        assert!(classify_trash_progress(&path, &evidence).is_ok());
    }

    #[test]
    fn trash_rejects_external_deletion_without_callback() {
        let path = PathBuf::from("target");
        let evidence = TrashProgressEvidence::default();

        assert!(matches!(
            classify_trash_progress(&path, &evidence),
            Err(CoreError::SecureTrashPostconditionFailed { path: actual }) if actual == path
        ));
    }

    #[test]
    fn trash_rejects_failed_or_wrong_target_callback() {
        let path = PathBuf::from("target");
        for evidence in [
            TrashProgressEvidence {
                post_delete_hresult: Some(-1),
                expected_target_callback: true,
            },
            TrashProgressEvidence {
                post_delete_hresult: Some(0),
                expected_target_callback: false,
            },
        ] {
            assert!(matches!(
                classify_trash_progress(&path, &evidence),
                Err(CoreError::SecureTrashPostconditionFailed { path: actual }) if actual == path
            ));
        }
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
    fn trash_moves_verified_target_to_recycle_bin() {
        let fixture = Fixture::new("trash");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("child.txt"), b"data").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&target_path));
        let secure = open_secure_target(&fixture.root, &target).unwrap();

        assert!(secure.capabilities().trash);
        match secure.move_to_trash() {
            Ok(()) => assert!(!target_path.exists()),
            Err(CoreError::SecureTrashUnavailable) => {
                eprintln!("environment unavailable: Windows Shell Trash backend");
                assert!(target_path.is_dir());
                assert_eq!(fs::read(target_path.join("child.txt")).unwrap(), b"data");
            }
            Err(error) => panic!("Windows Trash backend failed: {error:?}"),
        }
    }

    #[test]
    fn forced_trash_abort_preserves_target_contents() {
        let fixture = Fixture::new("forced-abort");
        let target_path = fixture.root.join("target");
        fs::create_dir(&target_path).unwrap();
        fs::write(target_path.join("original.txt"), b"original").unwrap();
        let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
            .with_identity(file_identity(&target_path));
        let secure = open_secure_target(&fixture.root, &target).unwrap();

        super::force_next_trash_abort();
        let result = secure.move_to_trash();
        assert!(
            matches!(result, Err(CoreError::SecureTrashAborted { ref path }) if path == &target_path),
            "unexpected forced-abort result: {result:?}"
        );
        assert_eq!(
            fs::read(target_path.join("original.txt")).unwrap(),
            b"original"
        );
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
            verify_shell_target_absent(&target, expected, &[]),
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

        assert!(verify_shell_target_absent(&target, expected, &[]).is_ok());
    }

    #[test]
    fn shell_postcondition_rejects_missing_ancestor_as_ambiguous() {
        let fixture = Fixture::new("postcondition-missing-ancestor");
        let ancestor = fixture.root.join("ancestor");
        let target = ancestor.join("target");
        fs::create_dir(&ancestor).unwrap();
        fs::create_dir(&target).unwrap();
        let opened = super::open_directory(&ancestor, &target).unwrap();
        let snapshot = AncestorSnapshot {
            identity: opened.identity,
            path: opened.path.clone(),
        };
        let expected = file_identity(&target).unwrap();
        drop(opened);
        fs::remove_dir(&target).unwrap();
        fs::remove_dir(&ancestor).unwrap();

        assert!(matches!(
            verify_shell_target_absent(&target, expected, &[snapshot]),
            Err(CoreError::SecureTrashPostconditionFailed { path }) if path == target
        ));
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
