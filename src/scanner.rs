use crate::error::CoreResult;
use crate::filesystem::{
    canonical_workspace, directory_size, is_protected_directory, read_entries,
};
use crate::model::{Category, ScanReport, ScanTarget};
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

    let mut targets = Vec::with_capacity(candidates.len());
    for path in candidates {
        targets.push(ScanTarget::new(
            path.clone(),
            Category::NodeModules,
            directory_size(&path)?,
        ));
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
    visit_directories(&root, providers, cancelled, &mut callback)?;
    Ok(())
}

fn visit_directories<F>(
    directory: &Path,
    providers: &ProviderSet,
    cancelled: &AtomicBool,
    callback: &mut F,
) -> CoreResult<bool>
where
    F: FnMut(&TraversalEntry) -> TraversalControl,
{
    if cancelled.load(Ordering::Acquire) {
        return Ok(true);
    }
    let mut entries = read_entries(directory)?;
    entries.sort_by_key(|left| left.file_name());
    for entry in entries {
        if cancelled.load(Ordering::Acquire) {
            return Ok(true);
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_protected_directory(&name) {
            continue;
        }
        let category = providers
            .scanners
            .iter()
            .find(|scanner| scanner.matches(&name) && scanner.confirm_context(&entry.path()))
            .map(|scanner| scanner.category());
        let traversal_entry = TraversalEntry {
            path: fs::canonicalize(entry.path())?,
            name,
            category,
        };
        match callback(&traversal_entry) {
            TraversalControl::Prune => continue,
            TraversalControl::Cancel => return Ok(true),
            TraversalControl::Continue => {}
        }
        if visit_directories(&entry.path(), providers, cancelled, callback)? {
            return Ok(true);
        }
    }
    Ok(false)
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
