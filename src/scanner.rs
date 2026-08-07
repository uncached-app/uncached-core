use crate::error::{CoreError, CoreResult};
use crate::filesystem::{
    canonical_workspace, directory_size, file_identity, is_protected_directory,
};
use crate::model::{Category, ScanReport, ScanTarget};
use jwalk::{Parallelism, WalkDir};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub trait Scanner: Send + Sync {
    fn category(&self) -> Category;
    fn matches(&self, name: &str) -> bool;
    fn confirm_context(&self, path: &Path) -> bool;
}

pub struct NodeModulesScanner;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalControl {
    Continue,
    Prune,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraversalEntry {
    pub path: PathBuf,
    pub name: String,
    pub category: Option<Category>,
    pub identity: Option<crate::model::FileIdentity>,
}

impl Scanner for NodeModulesScanner {
    fn category(&self) -> Category {
        Category::NodeModules
    }

    fn matches(&self, name: &str) -> bool {
        name == "node_modules"
    }

    fn confirm_context(&self, path: &Path) -> bool {
        !is_nested_node_modules(path)
            && !is_tooling_node_modules(path)
            && path
                .parent()
                .and_then(|parent| fs::symlink_metadata(parent.join("package.json")).ok())
                .is_some_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    }
}

pub struct ProviderSet {
    scanners: Vec<Box<dyn Scanner>>,
}

impl ProviderSet {
    pub fn free() -> Self {
        Self {
            scanners: vec![Box::new(NodeModulesScanner)],
        }
    }

    pub fn len(&self) -> usize {
        self.scanners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scanners.is_empty()
    }
}

pub fn scan_workspace(root: &Path, providers: &ProviderSet) -> CoreResult<ScanReport> {
    let root = canonical_workspace(root)?;
    let mut candidates = Vec::new();
    let cancelled = AtomicBool::new(false);
    traverse_workspace(&root, providers, &cancelled, |entry| {
        if let Some(Category::NodeModules) = entry.category {
            candidates.push(entry.path.clone());
            TraversalControl::Prune
        } else {
            TraversalControl::Continue
        }
    })?;
    candidates.sort();

    // Target sizing remains sequential so scan results preserve existing error semantics;
    // directory traversal and the later cleanup-specific sizing path are independently bounded.
    let mut targets = Vec::with_capacity(candidates.len());
    for path in candidates {
        targets.push(
            ScanTarget::new(path.clone(), Category::NodeModules, directory_size(&path)?)
                .with_identity(file_identity(&path)),
        );
    }
    let total_bytes = targets.iter().map(|target| target.size_bytes).sum();
    Ok(ScanReport {
        targets,
        total_bytes,
    })
}

pub fn traverse_workspace<F>(
    root: &Path,
    providers: &ProviderSet,
    cancelled: &AtomicBool,
    mut callback: F,
) -> CoreResult<()>
where
    F: FnMut(&TraversalEntry) -> TraversalControl,
{
    let root = canonical_workspace(root)?;
    if cancelled.load(Ordering::Acquire) {
        return Ok(());
    }

    let workers = std::thread::available_parallelism()
        .map(|count| count.get().clamp(1, 8))
        .unwrap_or(1);
    let mut entries = WalkDir::new(&root)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(Parallelism::RayonNewPool(workers))
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
                if is_protected_directory(&name) {
                    entry.read_children_path = None;
                    return false;
                }
                // Default provider target is pruned in the parallel walk itself. Custom
                // providers still get callback-level pruning after deterministic replay.
                let node_modules = NodeModulesScanner;
                if name == "node_modules" && node_modules.confirm_context(&entry.path()) {
                    entry.read_children_path = None;
                }
                true
            });
        })
        .into_iter()
        .map(|result| result.map_err(|error| CoreError::Io(error.into())))
        .collect::<CoreResult<Vec<_>>>()?;

    entries.sort_by_key(|entry| entry.path());
    let providers = &providers.scanners;
    let mut pruned = Vec::new();
    for entry in entries {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let path = entry.path();
        if pruned
            .iter()
            .any(|ancestor: &PathBuf| path.starts_with(ancestor))
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let category = providers
            .iter()
            .find(|scanner| scanner.matches(&name) && scanner.confirm_context(&path))
            .map(|scanner| scanner.category());
        let traversal_entry = TraversalEntry {
            path: fs::canonicalize(&path)?,
            name,
            category,
            identity: file_identity(&path),
        };
        match callback(&traversal_entry) {
            TraversalControl::Prune => pruned.push(path),
            TraversalControl::Cancel => return Ok(()),
            TraversalControl::Continue => {}
        }
    }
    Ok(())
}

fn is_nested_node_modules(path: &Path) -> bool {
    path.ancestors()
        .skip(1)
        .filter_map(Path::file_name)
        .any(|name| name.to_string_lossy().eq_ignore_ascii_case("node_modules"))
}

fn is_tooling_node_modules(path: &Path) -> bool {
    let parts = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>();
    parts.windows(5).any(|window| {
        window
            == [
                "visualstudio",
                "msbuild",
                "microsoft",
                "visualstudio",
                "nodejs",
            ]
    }) || parts.windows(2).any(|window| {
        window[0] == "nvm"
            && window[1].starts_with('v')
            && window[1][1..]
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
    })
}
