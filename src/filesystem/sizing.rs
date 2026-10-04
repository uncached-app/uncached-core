use crate::error::{CoreError, CoreResult};
use crate::parallel::worker_pool;
use jwalk::{Parallelism, WalkDir};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirectorySizeReport {
    /// Sum of file sizes that were read successfully.
    pub bytes: u64,
    /// Number of entries that could not be read, or size additions that overflowed.
    pub errors: u64,
    /// Newest modification time in the tree, in seconds since the Unix epoch.
    ///
    /// Answers how long ago a cache was last written to, which is the only
    /// cheap signal there is for whether anything still depends on it. Taken
    /// from files alone: a directory's own timestamp changes when its children
    /// are listed on some systems, and a tree with no readable file has no
    /// answer rather than a wrong one.
    pub newest_modified: Option<i64>,
}

impl DirectorySizeReport {
    /// Returns whether every entry contributed to the reported size.
    pub fn is_complete(&self) -> bool {
        self.errors == 0
    }
}

fn add_file_size(report: &mut DirectorySizeReport, bytes: u64) {
    if let Some(total) = report.bytes.checked_add(bytes) {
        report.bytes = total;
    } else {
        // Keep the partial total bounded, but make the loss visible to callers.
        report.bytes = u64::MAX;
        report.errors = report.errors.saturating_add(1);
    }
}

/// Keeps the newest of the times seen. A clock that went backwards before the
/// epoch is recorded as negative rather than discarded, so the comparison stays
/// a comparison of instants.
fn note_modified(report: &mut DirectorySizeReport, metadata: &std::fs::Metadata) {
    let Ok(modified) = metadata.modified() else {
        return;
    };
    let seconds = match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => since.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    };
    if report.newest_modified.is_none_or(|newest| seconds > newest) {
        report.newest_modified = Some(seconds);
    }
}

fn size_parallelism() -> Parallelism {
    match worker_pool() {
        // `busy_timeout: None` keeps a saturated pool from aborting the walk:
        // an aborted walk would report a silently truncated size.
        Some(pool) => Parallelism::RayonExistingPool {
            pool,
            busy_timeout: None,
        },
        None => Parallelism::Serial,
    }
}

pub fn directory_size_report(root: &Path) -> CoreResult<DirectorySizeReport> {
    directory_size_report_with_cancel(root, &AtomicBool::new(false))?.ok_or_else(|| {
        CoreError::Io(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "directory sizing cancelled",
        ))
    })
}

/// Sizes a directory tree and returns `None` when cancellation is observed.
/// Cancellation is checked before each entry is inspected, so callers do not
/// have to wait for the remaining tree to be walked.
pub fn directory_size_report_with_cancel(
    root: &Path,
    cancelled: &AtomicBool,
) -> CoreResult<Option<DirectorySizeReport>> {
    directory_size_report_skipping(root, &[], cancelled)
}

/// The same, without descending into the subtrees named.
///
/// Measuring a project used to walk it whole and then walk each artifact
/// directory again for its own figure, so `node_modules` and `target` were
/// visited twice — and in a typical project those are ninety-eight per cent of
/// the files. Sizing the artifacts once and the rest without them gives both
/// numbers for one pass over each file.
///
/// The doubling costs almost nothing on a warm macOS cache and is what makes
/// this take minutes on Windows, where a real-time scanner inspects every file
/// as it is opened. The waste was on every platform; only one of them notices.
///
/// `skip` holds directories, compared by exact path against what the walk is
/// about to descend into. Callers pass paths from the same traversal, so they
/// already agree on spelling.
pub fn directory_size_report_skipping(
    root: &Path,
    skip: &[PathBuf],
    cancelled: &AtomicBool,
) -> CoreResult<Option<DirectorySizeReport>> {
    let skip: std::collections::HashSet<PathBuf> = skip.iter().cloned().collect();
    WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(size_parallelism())
        .process_read_dir(move |_, _, _, children| {
            // Pruned here rather than filtered afterwards: the point is not to
            // read what is inside them.
            children.retain(|child| match child {
                Ok(entry) => !skip.contains(&entry.path()),
                Err(_) => true,
            });
        })
        .into_iter()
        .try_fold(DirectorySizeReport::default(), |mut report, entry| {
            if cancelled.load(Ordering::Acquire) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "directory sizing cancelled",
                ));
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    report.errors = report.errors.saturating_add(1);
                    return Ok(report);
                }
            };
            match entry.metadata() {
                Ok(metadata) if metadata.is_file() => {
                    add_file_size(&mut report, metadata.len());
                    note_modified(&mut report, &metadata);
                }
                Ok(_) => {}
                Err(_) => report.errors = report.errors.saturating_add(1),
            }
            Ok(report)
        })
        .map(Some)
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::Interrupted {
                Ok(None)
            } else {
                Err(CoreError::Io(error))
            }
        })
}

#[cfg(test)]
mod skipping_tests {
    use super::{directory_size_report, directory_size_report_skipping};
    use std::fs;
    use std::sync::atomic::AtomicBool;

    /// The property the optimisation rests on: measuring the artifacts once and
    /// the rest without them adds up to measuring everything. If it ever does
    /// not, a project's total silently loses whatever fell through the gap —
    /// which is worse than the slowness this replaced.
    #[test]
    fn what_is_skipped_plus_what_is_skipping_it_is_the_whole() {
        let root = std::env::temp_dir().join(format!("uncached-skip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let artifact = root.join("node_modules");
        fs::create_dir_all(artifact.join("deep").join("deeper")).expect("fixture");
        fs::create_dir_all(root.join("src")).expect("fixture");
        fs::write(root.join("src").join("main.rs"), vec![b'a'; 500]).expect("fixture");
        fs::write(root.join("README.md"), vec![b'b'; 300]).expect("fixture");
        fs::write(artifact.join("package.json"), vec![b'c'; 700]).expect("fixture");
        fs::write(
            artifact.join("deep").join("deeper").join("blob"),
            vec![b'd'; 1100],
        )
        .expect("fixture");

        let whole = directory_size_report(&root).expect("whole");
        let without = directory_size_report_skipping(
            &root,
            std::slice::from_ref(&artifact),
            &AtomicBool::new(false),
        )
        .expect("without")
        .expect("not cancelled");
        let only = directory_size_report(&artifact).expect("only");

        assert_eq!(
            without.bytes, 800,
            "the source files, and nothing under the artifact"
        );
        assert_eq!(only.bytes, 1800);
        assert_eq!(
            without.bytes + only.bytes,
            whole.bytes,
            "one pass over each file has to reach the same total as two"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// Skipping a path that is not there changes nothing, which is what lets a
    /// caller pass targets that vanished between the walk and the sizing.
    #[test]
    fn a_skip_that_matches_nothing_is_not_a_smaller_total() {
        let root = std::env::temp_dir().join(format!("uncached-skip-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("fixture");
        fs::write(root.join("file"), vec![b'x'; 42]).expect("fixture");

        let whole = directory_size_report(&root).expect("whole");
        let skipped = directory_size_report_skipping(
            &root,
            &[root.join("does-not-exist")],
            &AtomicBool::new(false),
        )
        .expect("skipped")
        .expect("not cancelled");

        assert_eq!(skipped.bytes, whole.bytes);
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{add_file_size, DirectorySizeReport};
    use std::fs;
    use std::path::{Path, PathBuf};

    /// The newest file in the tree is what says how long ago the cache was
    /// written to, and a cache nothing has touched in months is the one a user
    /// can decide about without opening it.
    #[test]
    fn sizing_reports_the_newest_file_in_the_tree() {
        let root = test_tree("newest");
        fs::create_dir_all(root.join("nested")).expect("fixture should be created");
        fs::write(root.join("old.bin"), b"old").expect("file should be written");
        fs::write(root.join("nested/new.bin"), b"new").expect("file should be written");

        let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(root.join("old.bin"))
            .expect("file should open")
            .set_modified(long_ago)
            .expect("modification time should be settable");

        let report = super::directory_size_report(&root).expect("fixture should be sized");
        let newest = report
            .newest_modified
            .expect("a tree with files has an age");

        assert!(
            newest > 1_000_000_000,
            "the newest file wins over the oldest one"
        );

        let empty = test_tree("empty");
        fs::create_dir_all(&empty).expect("fixture should be created");
        assert_eq!(
            super::directory_size_report(&empty)
                .expect("fixture should be sized")
                .newest_modified,
            None,
            "a tree with no readable file has no answer to give"
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&empty);
    }

    #[test]
    fn sizing_report_marks_partial_results_and_overflow() {
        let mut report = DirectorySizeReport {
            bytes: u64::MAX,
            errors: 0,
            newest_modified: None,
        };

        add_file_size(&mut report, 1);

        assert_eq!(report.bytes, u64::MAX);
        assert_eq!(report.errors, 1);
        assert!(!report.is_complete());
    }

    #[test]
    fn cancellable_sizing_stops_before_walking_when_requested() {
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let result = super::directory_size_report_with_cancel(Path::new("missing"), &cancelled);

        assert!(result.unwrap().is_none());
    }

    /// A directory of its own per case: `unique_test_path` is one path for the
    /// whole module, and another test leaves a file standing there.
    fn test_tree(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("uncached-core-size-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }
}
