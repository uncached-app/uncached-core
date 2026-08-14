use super::pruning::{should_prune_directory, PrunedPaths};
use super::{ProviderSet, TraversalControl, TraversalEntry};
use crate::error::CoreResult;
use crate::filesystem::{canonical_workspace, file_identity};
use crate::model::TraversalOutcome;
use jwalk::{Parallelism, WalkDir};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub fn traverse_workspace<F>(
    root: &Path,
    providers: &ProviderSet,
    cancelled: &AtomicBool,
    mut callback: F,
) -> CoreResult<TraversalOutcome>
where
    F: FnMut(&TraversalEntry) -> TraversalControl,
{
    let root = canonical_workspace(root)?;
    if cancelled.load(Ordering::Acquire) {
        return Ok(TraversalOutcome::Cancelled);
    }

    // Serial on purpose: the callback decides pruning after an entry is yielded,
    // and a parallel walker prefetches children before that decision arrives —
    // which is exactly the descent into confirmed artifact directories that this
    // traversal exists to avoid. Throughput here is bounded by the pruning, not
    // by the number of walker threads.
    let entries = WalkDir::new(&root)
        .sort(true)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(Parallelism::Serial)
        .process_read_dir(|_depth, _path, _state, children| {
            children.retain_mut(|result| {
                let Ok(entry) = result else {
                    return true;
                };
                let name = entry.file_name().to_string_lossy();
                if entry.path_is_symlink() || !entry.file_type().is_dir() {
                    entry.read_children_path = None;
                    return false;
                }
                if should_prune_directory(&name) {
                    entry.read_children_path = None;
                    return false;
                }
                true
            });
        })
        .into_iter();
    let mut pruned = PrunedPaths::default();
    let mut unreadable = 0usize;
    for result in entries {
        if cancelled.load(Ordering::Acquire) {
            return Ok(TraversalOutcome::Cancelled);
        }
        // A directory the operating system withholds is skipped, not fatal: on a
        // whole-disk scan one protected folder would otherwise cost every
        // artifact found before it. The count says the reading is partial.
        let Ok(entry) = result else {
            unreadable += 1;
            continue;
        };
        let path = entry.path();
        if pruned.contains_ancestor(&path) {
            continue;
        }
        // Pruned rather than skipped: an excluded directory is not searched
        // either, so a rule protects everything beneath it without the user
        // having to say so twice.
        if providers.excludes(&path) {
            pruned.insert(path);
            continue;
        }
        // The walker reports a directory it could not open on the directory
        // itself, so an unreadable subtree arrives as an entry carrying an error
        // rather than as a failed one. Counted after the pruning, so a subtree
        // nobody wanted to search is not reported as one that was lost.
        if entry.read_children_error.is_some() {
            unreadable += 1;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let category = providers.classify(&name, &path);
        let identity = category.as_ref().and_then(|_| file_identity(&path));
        let traversal_entry = TraversalEntry {
            path: path.clone(),
            name,
            category,
            identity,
        };
        match callback(&traversal_entry) {
            TraversalControl::Prune => pruned.insert(path),
            TraversalControl::Cancel => return Ok(TraversalOutcome::Cancelled),
            TraversalControl::Continue => {}
        }
    }
    Ok(TraversalOutcome::Completed { unreadable })
}
