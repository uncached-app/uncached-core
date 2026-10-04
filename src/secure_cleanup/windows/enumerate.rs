//! Collecting every entry under a target before any deletion, bounded in depth
//! and count so a pathological tree fails closed.

use super::*;

/// Directory nesting a secure open accepts. Deeper trees are refused instead of
/// walked, so a pathological layout cannot exhaust memory during collection.
pub(super) const MAX_SECURE_TREE_DEPTH: usize = 512;
/// Handles retained at once by a single secure open. The whole subtree is opened
/// and verified before any deletion, so this caps kernel handle consumption and
/// fails closed instead of exhausting the process handle table.
pub(super) const MAX_SECURE_TREE_ENTRIES: usize = 500_000;

/// A directory handle being handed to a worker thread. `HANDLE` is a plain
/// kernel handle value with no thread affinity: it is valid process-wide, and
/// the owning `Entry` outlives the expansion that borrows it.
#[derive(Clone, Copy)]
pub(super) struct DirectoryToExpand {
    pub(super) handle: HANDLE,
    pub(super) volume_serial: u64,
}

// SAFETY: file handles carry no thread affinity; only COM interfaces do, and
// none cross this boundary. Ownership stays with the `Entry` on the main thread.
unsafe impl Send for DirectoryToExpand {}
unsafe impl Sync for DirectoryToExpand {}

/// Collects the target subtree level by level, expanding every directory of a
/// level in parallel. Two invariants matter and both survive the parallelism:
/// entries stay in an order where a parent precedes its descendants, which is
/// what `delete_permanently` relies on when it drains the collection in reverse;
/// and the result is deterministic, because each level is appended in frontier
/// order rather than completion order. Iterative, so nesting cannot exhaust the
/// stack.
pub(super) fn collect_entries(root: Entry, entries: &mut Vec<Entry>) -> CoreResult<()> {
    let mut frontier = Vec::new();
    push_entry(root, entries, &mut frontier, 0)?;

    let mut depth = 0;
    while !frontier.is_empty() {
        depth += 1;
        let level = expand_level(&frontier)?;
        frontier.clear();
        for children in level {
            for child in children {
                push_entry(child, entries, &mut frontier, depth)?;
            }
        }
    }
    Ok(())
}

/// Enumerates one level. Falls back to sequential enumeration when no worker
/// pool is available, so behaviour never depends on pool construction.
pub(super) fn expand_level(
    frontier: &[(DirectoryToExpand, PathBuf)],
) -> CoreResult<Vec<Vec<Entry>>> {
    let enumerate = |(directory, path): &(DirectoryToExpand, PathBuf)| {
        enumerate_directory(directory.handle, path, directory.volume_serial)
    };

    let results: Vec<CoreResult<Vec<Entry>>> = match crate::parallel::worker_pool() {
        Some(pool) => pool.install(|| {
            use jwalk::rayon::prelude::*;
            frontier.par_iter().map(enumerate).collect()
        }),
        None => frontier.iter().map(enumerate).collect(),
    };

    // Report the first failure in frontier order, not in completion order, so a
    // failing tree always produces the same error.
    results.into_iter().collect()
}

pub(super) fn push_entry(
    entry: Entry,
    entries: &mut Vec<Entry>,
    frontier: &mut Vec<(DirectoryToExpand, PathBuf)>,
    depth: usize,
) -> CoreResult<()> {
    if entries.len() >= MAX_SECURE_TREE_ENTRIES {
        return Err(CoreError::SecureTargetTooLarge {
            path: entry.path,
            limit: MAX_SECURE_TREE_ENTRIES,
        });
    }
    if entry.is_directory {
        if depth >= MAX_SECURE_TREE_DEPTH {
            return Err(CoreError::SecureTargetTooDeep {
                path: entry.path,
                limit: MAX_SECURE_TREE_DEPTH,
            });
        }
        frontier.push((
            DirectoryToExpand {
                handle: entry.handle.0,
                volume_serial: entry.identity.volume_serial,
            },
            entry.path.clone(),
        ));
    }
    entries.push(entry);
    Ok(())
}

pub(super) fn enumerate_directory(
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
                let entry = open_entry(&path, &path, AccessProfile::Delete)?;
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

pub(super) fn map_disposition_error(path: &Path, source: std::io::Error) -> CoreError {
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
