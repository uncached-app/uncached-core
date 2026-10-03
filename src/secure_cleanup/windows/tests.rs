use super::{
    classify_trash_progress, identity_matches, map_aborted_operations, map_disposition_error,
    map_trash_error, open_error_path_with_source, open_secure_target, os_string_from_wide,
    parse_directory_records, progress_sink_add_ref, progress_sink_release,
    revalidate_shell_ancestor, revalidate_shell_target, shell_wide_path,
    verify_shell_target_absent, AncestorSnapshot, ProgressSink, TrashProgressEvidence,
};
use crate::error::CoreError;
use crate::filesystem::file_identity;
use crate::model::{Category, ScanTarget};
use crate::secure_cleanup::CleanupIntent;
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
fn empty_trash_target_returns_typed_initialization_error() {
    let target = super::WindowsTarget {
        _ancestors: Vec::new(),
        entries: Vec::new(),
        intent: CleanupIntent::Trash,
    };

    assert!(matches!(
        target.move_to_trash(),
        Err(CoreError::SecureTrashInitializationFailed { .. })
    ));
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
        -2_147_024_776, // HRESULT_FROM_WIN32(ERROR_CALL_NOT_IMPLEMENTED)
    ] {
        assert!(matches!(
            map_trash_error(&path, "CoCreateInstance", hresult),
            CoreError::SecureTrashUnavailable
        ));
    }
}

#[test]
fn file_not_found_does_not_map_to_typed_unavailable_error() {
    let error = map_trash_error(&PathBuf::from("target"), "CoCreateInstance", -2_147_024_894);

    assert!(matches!(
        error,
        CoreError::SecureTrashOperationFailed { .. }
    ));
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

/// Test-only probe: production always requests enumeration access first and
/// falls back inside `open_entry`.
fn open_entry_with_access(
    path: &std::path::Path,
    display: &std::path::Path,
    access: u32,
    can_enumerate: bool,
) -> crate::error::CoreResult<super::Entry> {
    let wide = super::wide_path(path);
    // SAFETY: wide is NUL-terminated; all other arguments are fixed valid flags.
    let handle = unsafe {
        windows_sys::Win32::Storage::FileSystem::CreateFileW(
            wide.as_ptr(),
            access,
            windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ
                | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE
                | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE,
            std::ptr::null(),
            windows_sys::Win32::Storage::FileSystem::OPEN_EXISTING,
            windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS
                | windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT
                | windows_sys::Win32::Storage::FileSystem::FILE_FLAG_POSIX_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(CoreError::SecureTargetOpenFailed {
            path: display.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    super::open_entry_with_handle(handle, path, display, can_enumerate, None)
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
    open_secure_target(&fixture.root, &target, CleanupIntent::Permanent)
        .expect("verified target should open");
    let capabilities = crate::cleanup_capabilities();
    assert!(capabilities.permanent_cleanup);
    assert_eq!(capabilities.move_to_trash, super::shell_backend_available());
}

#[test]
fn trash_moves_verified_target_to_recycle_bin() {
    let fixture = Fixture::new("trash");
    let target_path = fixture.root.join("target");
    fs::create_dir(&target_path).unwrap();
    fs::write(target_path.join("child.txt"), b"data").unwrap();
    let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
        .with_identity(file_identity(&target_path));
    let secure = open_secure_target(&fixture.root, &target, CleanupIntent::Trash).unwrap();

    assert_eq!(
        crate::cleanup_capabilities().move_to_trash,
        super::shell_backend_available()
    );
    match secure.move_to_trash() {
        Ok(()) => assert!(!target_path.exists()),
        Err(error) => {
            assert!(matches!(
                &error,
                CoreError::SecureTrashUnavailable
                    | CoreError::SecureTrashInitializationFailed { .. }
                    | CoreError::SecureTrashOperationFailed { .. }
                    | CoreError::SecureTrashPostconditionFailed { .. }
            ));
            eprintln!("Windows Shell Trash returned an environment/operation error: {error:?}");
            if matches!(
                &error,
                CoreError::SecureTrashUnavailable
                    | CoreError::SecureTrashInitializationFailed { .. }
            ) {
                assert!(target_path.is_dir());
                assert_eq!(fs::read(target_path.join("child.txt")).unwrap(), b"data");
            }
        }
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
    let secure = open_secure_target(&fixture.root, &target, CleanupIntent::Trash).unwrap();

    super::force_next_trash_abort();
    let result = secure.move_to_trash();
    match result {
        Err(CoreError::SecureTrashAborted { path }) => assert_eq!(path, target_path),
        Err(CoreError::SecureTrashUnavailable) => {}
        Err(error) => panic!("unexpected typed Trash result: {error:?}"),
        Ok(()) => panic!("forced abort must not report success"),
    }
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
    let target = ScanTarget::new(original.clone(), Category::NodeModules, 0).with_identity(Some(
        crate::model::FileIdentity {
            volume_serial: 0,
            file_id: [0; 16],
        },
    ));
    assert!(matches!(
        open_secure_target(&fixture.root, &target, CleanupIntent::Permanent),
        Err(CoreError::TargetIdentityChanged { .. })
    ));
    let identity = file_identity(&original);
    fs::rename(&original, fixture.root.join("old")).unwrap();
    fs::create_dir(&original).unwrap();
    let replaced = ScanTarget::new(original, Category::NodeModules, 0).with_identity(identity);
    assert!(matches!(
        open_secure_target(&fixture.root, &replaced, CleanupIntent::Permanent),
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
        open_secure_target(&fixture.root, &direct, CleanupIntent::Permanent),
        Err(CoreError::TargetReparsePoint { .. })
    ));
    fs::create_dir(fixture.root.join("real").join("target")).unwrap();
    if let Err(error) = std::os::windows::fs::symlink_dir("real", fixture.root.join("ancestor")) {
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
        open_secure_target(&fixture.root, &ancestor, CleanupIntent::Permanent),
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

    let opened = super::open_directory(&ancestor, &target, super::AccessProfile::Inspect).unwrap();
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
    let opened = super::open_directory(&ancestor, &target, super::AccessProfile::Inspect).unwrap();
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
fn trash_open_succeeds_while_another_handle_denies_delete_sharing() {
    // Reproduces a directory another process is using: a build directory, an
    // editor, or a process whose working directory it is. Trash cleanup only
    // pins the target, so it must not fail; permanent cleanup needs DELETE
    // and is expected to report the sharing violation.
    let fixture = Fixture::new("sharing-violation");
    let target_path = fixture.root.join("target");
    fs::create_dir(&target_path).unwrap();
    fs::write(target_path.join("child.txt"), b"data").unwrap();
    let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
        .with_identity(file_identity(&target_path));
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = target_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: wide is NUL-terminated and the flags are documented values.
    let blocking = unsafe {
        windows_sys::Win32::Storage::FileSystem::CreateFileW(
            wide.as_ptr(),
            // Mirrors how a process holds its working directory: read and
            // list access, shared for read and write but never for delete.
            windows_sys::Win32::Storage::FileSystem::FILE_LIST_DIRECTORY
                | windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES,
            windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ
                | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE,
            std::ptr::null(),
            windows_sys::Win32::Storage::FileSystem::OPEN_EXISTING,
            windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(
        blocking,
        windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
        "test must hold a handle that denies delete sharing"
    );

    let trash = open_secure_target(&fixture.root, &target, CleanupIntent::Trash);
    let permanent = open_secure_target(&fixture.root, &target, CleanupIntent::Permanent);

    // SAFETY: the handle came from a successful CreateFileW above.
    unsafe { windows_sys::Win32::Foundation::CloseHandle(blocking) };

    assert!(trash.is_ok(), "unexpected Trash open failure: {trash:?}");
    match permanent {
        Err(CoreError::SecureTargetOpenFailed { source, .. }) => {
            assert_eq!(source.raw_os_error(), Some(32));
        }
        other => panic!("permanent open must report the sharing violation: {other:?}"),
    }
}

#[test]
fn permanent_cleanup_removes_a_nested_tree_in_child_before_parent_order() {
    let fixture = Fixture::new("nested-delete");
    let target_path = fixture.root.join("target");
    let deep = target_path.join("a").join("b").join("c");
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("leaf.txt"), b"leaf").unwrap();
    for sibling in 0..8 {
        let branch = target_path.join(format!("branch-{sibling}"));
        fs::create_dir_all(branch.join("nested")).unwrap();
        fs::write(branch.join("nested").join("file.txt"), b"data").unwrap();
        fs::write(branch.join("root.txt"), b"data").unwrap();
    }
    let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
        .with_identity(file_identity(&target_path));

    let secure = open_secure_target(&fixture.root, &target, CleanupIntent::Permanent).unwrap();
    secure.delete_permanently().unwrap();

    assert!(!target_path.exists());
}

#[test]
fn permanent_cleanup_removes_verified_tree_through_retained_handles() {
    let fixture = Fixture::new("handle-delete");
    let target_path = fixture.root.join("target");
    fs::create_dir(&target_path).unwrap();
    fs::write(target_path.join("original.txt"), b"original").unwrap();
    let target = ScanTarget::new(target_path.clone(), Category::NodeModules, 0)
        .with_identity(file_identity(&target_path));
    let secure = open_secure_target(&fixture.root, &target, CleanupIntent::Permanent).unwrap();
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
    let secure = open_secure_target(&fixture.root, &target, CleanupIntent::Permanent).unwrap();

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
