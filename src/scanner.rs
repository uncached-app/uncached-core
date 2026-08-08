use crate::error::{CoreError, CoreResult};
use crate::filesystem::{
    canonical_workspace, directory_size, file_identity, is_protected_directory,
};
use crate::model::{Category, ScanOutcome, ScanReport, ScanTarget, TraversalOutcome};
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
pub struct CargoTargetScanner;
pub struct NextCacheScanner;
pub struct TurboCacheScanner;
pub struct PyCacheScanner;
pub struct GradleScanner;

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
    pub fn new() -> Self {
        Self {
            scanners: Vec::new(),
        }
    }

    pub fn with_scanner(scanner: impl Scanner + 'static) -> Self {
        Self {
            scanners: vec![Box::new(scanner)],
        }
    }

    pub fn add_scanner(&mut self, scanner: impl Scanner + 'static) {
        self.scanners.push(Box::new(scanner));
    }

    pub fn free() -> Self {
        Self {
            scanners: vec![Box::new(NodeModulesScanner)],
        }
    }

    pub fn all() -> Self {
        Self {
            scanners: vec![
                Box::new(NodeModulesScanner),
                Box::new(CargoTargetScanner),
                Box::new(NextCacheScanner),
                Box::new(TurboCacheScanner),
                Box::new(PyCacheScanner),
                Box::new(GradleScanner),
            ],
        }
    }

    pub fn len(&self) -> usize {
        self.scanners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scanners.is_empty()
    }

    pub fn scanner_for(&self, category: Category) -> Option<&dyn Scanner> {
        self.scanners
            .iter()
            .find(|scanner| scanner.category() == category)
            .map(Box::as_ref)
    }
}

impl Scanner for CargoTargetScanner {
    fn category(&self) -> Category {
        Category::CargoTarget
    }

    fn matches(&self, name: &str) -> bool {
        name == "target"
    }

    fn confirm_context(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| parent.join("Cargo.toml").exists())
    }
}

impl Scanner for NextCacheScanner {
    fn category(&self) -> Category {
        Category::NextJs
    }

    fn matches(&self, name: &str) -> bool {
        name == ".next"
    }

    fn confirm_context(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| parent.join("package.json").exists())
    }
}

impl Scanner for TurboCacheScanner {
    fn category(&self) -> Category {
        Category::Turbo
    }

    fn matches(&self, name: &str) -> bool {
        name == ".turbo"
    }

    fn confirm_context(&self, path: &Path) -> bool {
        path.parent().is_some_and(|parent| {
            parent.join("package.json").exists() || parent.join("turbo.json").exists()
        })
    }
}

impl Scanner for PyCacheScanner {
    fn category(&self) -> Category {
        Category::PyCache
    }

    fn matches(&self, name: &str) -> bool {
        matches!(name, "__pycache__" | ".pytest_cache" | ".mypy_cache")
    }

    fn confirm_context(&self, _path: &Path) -> bool {
        true
    }
}

impl Scanner for GradleScanner {
    fn category(&self) -> Category {
        Category::Gradle
    }

    fn matches(&self, name: &str) -> bool {
        name == ".gradle"
    }

    fn confirm_context(&self, _path: &Path) -> bool {
        true
    }
}

impl Default for ProviderSet {
    fn default() -> Self {
        Self::new()
    }
}

pub fn scan_workspace(
    root: &Path,
    providers: &ProviderSet,
    cancelled: &AtomicBool,
) -> CoreResult<ScanOutcome> {
    scan_workspace_with_sizing(root, providers, cancelled, directory_size)
}

pub fn scan_workspace_with_sizing<S>(
    root: &Path,
    providers: &ProviderSet,
    cancelled: &AtomicBool,
    mut size: S,
) -> CoreResult<ScanOutcome>
where
    S: FnMut(&Path) -> CoreResult<u64>,
{
    let root = canonical_workspace(root)?;
    let mut candidates = Vec::new();
    let outcome = traverse_workspace(&root, providers, cancelled, |entry| {
        if let Some(category) = entry.category.clone() {
            candidates.push((entry.path.clone(), category, entry.identity));
            TraversalControl::Prune
        } else {
            TraversalControl::Continue
        }
    })?;
    if outcome == TraversalOutcome::Cancelled {
        return Ok(ScanOutcome::Cancelled);
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));

    let mut targets = Vec::with_capacity(candidates.len());
    for (path, category, identity) in candidates {
        if cancelled.load(Ordering::Acquire) {
            return Ok(ScanOutcome::Cancelled);
        }
        let size_bytes = size(&path)?;
        if cancelled.load(Ordering::Acquire) {
            return Ok(ScanOutcome::Cancelled);
        }
        targets.push(ScanTarget::new(path, category, size_bytes).with_identity(identity));
    }
    let total_bytes = targets.iter().map(|target| target.size_bytes).sum();
    Ok(ScanOutcome::Completed(ScanReport {
        targets,
        total_bytes,
    }))
}

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
                if is_protected_directory(&name) {
                    entry.read_children_path = None;
                    return false;
                }
                true
            });
        })
        .into_iter();
    let providers = &providers.scanners;
    let mut pruned = Vec::new();
    for result in entries {
        if cancelled.load(Ordering::Acquire) {
            return Ok(TraversalOutcome::Cancelled);
        }
        let entry = result.map_err(|error| CoreError::Io(error.into()))?;
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
            path: path.clone(),
            name,
            category,
            identity: file_identity(&path),
        };
        match callback(&traversal_entry) {
            TraversalControl::Prune => pruned.push(path),
            TraversalControl::Cancel => return Ok(TraversalOutcome::Cancelled),
            TraversalControl::Continue => {}
        }
    }
    Ok(TraversalOutcome::Completed)
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
