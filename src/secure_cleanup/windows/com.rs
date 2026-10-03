//! The COM plumbing: interface ids, the raw vtable layouts, and the owners that
//! release what they hold.

use super::*;

pub(super) const IID_ISHELL_ITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
pub(super) const IID_IFILE_OPERATION: GUID =
    GUID::from_u128(0x947aab5f_0a5c_4c13_b4d6_4bf7836fc9f8);
pub(super) const IID_IFILE_OPERATION_PROGRESS_SINK: GUID =
    GUID::from_u128(0x04b0f1a7_9490_44bc_96e1_4296a31252e2);
pub(super) const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);

#[repr(C)]
pub(super) struct IUnknownVtbl {
    pub(super) query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub(super) add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub(super) release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
pub(super) struct IFileOperationVtbl {
    pub(super) base: IUnknownVtbl,
    pub(super) advise: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32) -> HRESULT,
    pub(super) unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    pub(super) set_operation_flags: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    pub(super) set_progress_message: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub(super) set_progress_dialog: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) set_properties: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) set_owner_window: unsafe extern "system" fn(*mut c_void, isize) -> HRESULT,
    pub(super) apply_properties_to_item:
        unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) apply_properties_to_items:
        unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) rename_item:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *const u16, *mut c_void) -> HRESULT,
    pub(super) rename_items:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *const u16) -> HRESULT,
    pub(super) move_item: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const u16,
        *mut c_void,
    ) -> HRESULT,
    pub(super) move_items:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    pub(super) copy_item: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const u16,
        *mut c_void,
    ) -> HRESULT,
    pub(super) copy_items:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    pub(super) delete_item:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    pub(super) delete_items: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub(super) new_item: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *const u16,
        *const u16,
        *mut c_void,
    ) -> HRESULT,
    pub(super) perform_operations: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub(super) get_any_operations_aborted:
        unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
}

#[repr(C)]
pub(super) struct ShellItemVtbl {
    pub(super) base: IUnknownVtbl,
    pub(super) bind_to_handler: *const c_void,
    pub(super) get_parent: *const c_void,
    pub(super) get_display_name:
        unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> HRESULT,
}

#[derive(Debug)]
pub(super) struct ComPtr<T> {
    pub(super) ptr: *mut c_void,
    pub(super) _interface: PhantomData<T>,
}

impl<T> ComPtr<T> {
    pub(super) unsafe fn from_raw(ptr: *mut c_void) -> Self {
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

pub(super) struct ComApartment;

impl ComApartment {
    pub(super) fn initialize_sta() -> CoreResult<Self> {
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

// windows-sys exposes these Shell COM interfaces through their ABI, not typed wrappers.
pub(super) enum IShellItemTag {}
pub(super) enum IFileOperationTag {}

pub(super) fn hresult_error(result: HRESULT) -> std::io::Error {
    std::io::Error::from_raw_os_error(result)
}

pub(super) fn shell_wide_path(path: &Path) -> Vec<u16> {
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
