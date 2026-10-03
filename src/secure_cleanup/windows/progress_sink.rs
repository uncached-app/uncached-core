//! The `IFileOperationProgressSink` the Shell calls back into, and the evidence
//! it records about whether the expected target was really deleted.

use super::*;

#[repr(C)]
pub(super) struct IFileOperationProgressSinkVtbl {
    pub(super) base: IUnknownVtbl,
    pub(super) start_operations: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub(super) finish_operations: unsafe extern "system" fn(*mut c_void, HRESULT) -> HRESULT,
    pub(super) pre_rename_item:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *const u16) -> HRESULT,
    pub(super) post_rename_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *const u16,
        HRESULT,
        *mut c_void,
    ) -> HRESULT,
    pub(super) pre_move_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *const u16,
    ) -> HRESULT,
    pub(super) post_move_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *const u16,
        HRESULT,
        *mut c_void,
    ) -> HRESULT,
    pub(super) pre_copy_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *const u16,
    ) -> HRESULT,
    pub(super) post_copy_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *const u16,
        HRESULT,
        *mut c_void,
    ) -> HRESULT,
    pub(super) pre_delete_item: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT,
    pub(super) post_delete_item:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, HRESULT, *mut c_void) -> HRESULT,
    pub(super) pre_new_item:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *const u16) -> HRESULT,
    pub(super) post_new_item: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *const u16,
        *const u16,
        u32,
        HRESULT,
        *mut c_void,
    ) -> HRESULT,
    pub(super) update_progress: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
    pub(super) reset_timer: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub(super) pause_timer: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub(super) resume_timer: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

#[derive(Debug, Default)]
pub(super) struct TrashProgressEvidence {
    pub(super) post_delete_hresult: Option<HRESULT>,
    pub(super) expected_target_callback: bool,
}

pub(super) fn classify_trash_progress(
    path: &Path,
    evidence: &TrashProgressEvidence,
) -> CoreResult<()> {
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
pub(super) struct ProgressSink {
    pub(super) vtable: *const IFileOperationProgressSinkVtbl,
    pub(super) references: std::sync::atomic::AtomicU32,
    pub(super) expected_path: PathBuf,
    pub(super) expected_item: *mut c_void,
    pub(super) evidence: Arc<Mutex<TrashProgressEvidence>>,
}

unsafe impl Send for ProgressSink {}
unsafe impl Sync for ProgressSink {}

pub(super) static PROGRESS_SINK_VTABLE: IFileOperationProgressSinkVtbl =
    IFileOperationProgressSinkVtbl {
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
    pub(super) fn allocate(
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

pub(super) unsafe fn progress_sink_from_raw<'a>(raw: *mut c_void) -> &'a ProgressSink {
    // SAFETY: COM calls provide the interface pointer originally returned by ProgressSink::new.
    unsafe { &*(raw.cast::<ProgressSink>()) }
}

pub(super) unsafe extern "system" fn progress_sink_query_interface(
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

pub(super) unsafe fn guid_matches(left: *const GUID, right: &GUID) -> bool {
    // SAFETY: caller validates left is non-null before comparing COM identifiers.
    unsafe { *left }.data1 == right.data1
        && unsafe { *left }.data2 == right.data2
        && unsafe { *left }.data3 == right.data3
        && unsafe { *left }.data4 == right.data4
}

pub(super) unsafe extern "system" fn progress_sink_add_ref(raw: *mut c_void) -> u32 {
    // SAFETY: raw is a live COM sink while AddRef is callable.
    unsafe { progress_sink_from_raw(raw) }
        .references
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        + 1
}

pub(super) unsafe extern "system" fn progress_sink_release(raw: *mut c_void) -> u32 {
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

pub(super) unsafe extern "system" fn progress_sink_ok(_raw: *mut c_void) -> HRESULT {
    0
}

pub(super) unsafe extern "system" fn progress_sink_finish(
    _raw: *mut c_void,
    _result: HRESULT,
) -> HRESULT {
    0
}

pub(super) unsafe extern "system" fn progress_sink_pre_rename(
    _raw: *mut c_void,
    _flags: u32,
    _item: *mut c_void,
    _name: *const u16,
) -> HRESULT {
    0
}

pub(super) unsafe extern "system" fn progress_sink_ok_post_rename(
    _raw: *mut c_void,
    _flags: u32,
    _item: *mut c_void,
    _name: *const u16,
    _result: HRESULT,
    _created: *mut c_void,
) -> HRESULT {
    0
}

pub(super) unsafe extern "system" fn progress_sink_pre_move(
    _raw: *mut c_void,
    _flags: u32,
    _item: *mut c_void,
    _folder: *mut c_void,
    _name: *const u16,
) -> HRESULT {
    0
}

pub(super) unsafe extern "system" fn progress_sink_pre_new(
    _raw: *mut c_void,
    _flags: u32,
    _folder: *mut c_void,
    _name: *const u16,
) -> HRESULT {
    0
}

pub(super) unsafe extern "system" fn progress_sink_ok_post_move(
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

pub(super) unsafe extern "system" fn progress_sink_pre_delete(
    _raw: *mut c_void,
    _flags: u32,
    _item: *mut c_void,
) -> HRESULT {
    0
}

pub(super) unsafe extern "system" fn progress_sink_post_delete(
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

pub(super) fn shell_paths_equal(left: &Path, right: &Path) -> bool {
    shell_wide_path(left) == shell_wide_path(right)
}

pub(super) unsafe fn shell_item_path(item: *mut c_void) -> Option<PathBuf> {
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

pub(super) unsafe extern "system" fn progress_sink_ok_post_new(
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

pub(super) unsafe extern "system" fn progress_sink_update(
    _raw: *mut c_void,
    _total: u32,
    _done: u32,
) -> HRESULT {
    0
}
