use super::pruning::{should_prune_directory, PrunedPaths};
use super::{ProviderSet, TraversalControl, TraversalEntry};
use crate::error::{CoreError, CoreResult};
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
    for result in entries {
        if cancelled.load(Ordering::Acquire) {
            return Ok(TraversalOutcome::Cancelled);
        }
        let entry = result.map_err(|error| CoreError::Io(error.into()))?;
        let path = entry.path();
        if pruned.contains_ancestor(&path) {
            continue;
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
    Ok(TraversalOutcome::Completed)
}
