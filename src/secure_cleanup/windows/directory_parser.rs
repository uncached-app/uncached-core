use std::ffi::OsString;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStringExt;
use windows_sys::Win32::Storage::FileSystem::FILE_ID_EXTD_DIR_INFO;

#[derive(Debug)]
pub(super) struct ParsedDirectoryRecord {
    pub(super) name: Vec<u16>,
    pub(super) file_id: [u8; 16],
}

pub(super) fn parse_directory_records(
    buffer: &[u8],
) -> Result<Vec<ParsedDirectoryRecord>, std::io::Error> {
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
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| u16::from_le_bytes(*bytes))
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

pub(super) fn os_string_from_wide(name: &[u16]) -> OsString {
    OsString::from_wide(name)
}

fn read_unaligned_at<T: Copy>(buffer: &[u8], offset: usize) -> T {
    // SAFETY: callers validate field bounds before reading; unaligned access is intentional.
    unsafe { std::ptr::read_unaligned(buffer.as_ptr().add(offset).cast()) }
}

fn invalid_directory_record(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}
