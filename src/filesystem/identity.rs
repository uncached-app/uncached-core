use crate::model::FileIdentity;
use std::path::Path;

pub fn file_identity(path: &Path) -> Option<FileIdentity> {
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path).ok()?;
        let mut file_id = [0; 16];
        file_id[..8].copy_from_slice(&metadata.ino().to_le_bytes());
        Some(FileIdentity {
            volume_serial: metadata.dev(),
            file_id,
        })
    }
    #[cfg(windows)]
    {
        windows_file_identity(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> Option<FileIdentity> {
    use std::fs;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, PathBuf};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FileAttributeTagInfo, FileIdInfo, GetFileAttributesW,
        GetFileInformationByHandleEx, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_128, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    let mut component_path = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                component_path.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => component_path.push(component.as_os_str()),
            Component::Normal(name) => {
                component_path.push(name);
                let metadata = fs::symlink_metadata(&component_path).ok()?;
                if metadata.file_type().is_symlink() {
                    return None;
                }
                let component_path_wide: Vec<u16> = component_path
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let attributes = unsafe { GetFileAttributesW(component_path_wide.as_ptr()) };
                if attributes == u32::MAX || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return None;
                }
            }
        }
    }

    let path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let _handle = Handle(handle);

    let mut attributes = FILE_ATTRIBUTE_TAG_INFO {
        FileAttributes: 0,
        ReparseTag: 0,
    };
    let attributes_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut attributes as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } != 0;
    if !attributes_ok
        || attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return None;
    }

    let mut information = FILE_ID_INFO {
        VolumeSerialNumber: 0,
        FileId: FILE_ID_128 {
            Identifier: [0; 16],
        },
    };
    let information_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } != 0;
    if !information_ok {
        return None;
    }

    Some(FileIdentity {
        volume_serial: information.VolumeSerialNumber,
        file_id: information.FileId.Identifier,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::file_identity;
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;

    #[test]
    fn file_identity_maps_unix_device_and_inode() {
        let path = unique_test_path();
        fs::write(&path, b"identity").expect("test file should be created");

        let metadata = fs::symlink_metadata(&path).expect("test file metadata should exist");
        let identity = file_identity(&path).expect("test file should have identity");

        assert_eq!(identity.volume_serial, metadata.dev());
        assert_eq!(identity.file_id[..8], metadata.ino().to_le_bytes());
        assert_eq!(identity.file_id[8..], [0; 8]);

        fs::remove_file(path).expect("test file should be removed");
    }

    fn unique_test_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "uncached-core-file-identity-{}",
            std::process::id()
        ))
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::file_identity;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    #[test]
    fn file_identity_reads_real_directory_identity() {
        let path = fixture_directory("stable");
        let first = file_identity(&path).expect("directory should have identity");
        let second = file_identity(&path).expect("directory identity should remain readable");

        assert_eq!(first, second);
        cleanup_fixture(path);
    }

    #[test]
    fn file_identity_returns_none_for_missing_directory() {
        let path = fixture_path("missing");

        assert_eq!(file_identity(&path), None);
    }

    #[test]
    fn file_identity_returns_none_when_parent_is_a_reparse_point() {
        let real = fixture_path("reparse-real");
        let link = fixture_path("reparse-link");
        let target = link.join("target");
        fs::create_dir_all(real.join("target")).expect("real target should be created");
        let junction_status = Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&link)
            .arg(&real)
            .status()
            .expect("junction command should start");
        if !junction_status.success() {
            fs::remove_dir_all(real).expect("real fixture should be removed");
            panic!("junction creation unsupported: mklink /J exited with {junction_status}");
        }

        assert_eq!(file_identity(&target), None);

        fs::remove_dir(link).expect("reparse fixture should be removed");
        fs::remove_dir_all(real).expect("real fixture should be removed");
    }

    fn fixture_directory(label: &str) -> PathBuf {
        let path = fixture_path(label);
        fs::create_dir(&path).expect("fixture directory should be created");
        path
    }

    fn fixture_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "uncached-core-file-identity-{label}-{}",
            std::process::id()
        ))
    }

    fn cleanup_fixture(path: PathBuf) {
        fs::remove_dir(path).expect("fixture directory should be removed");
    }
}
