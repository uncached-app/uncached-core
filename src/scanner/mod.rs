pub(crate) mod providers;
mod pruning;
mod traversal;

use crate::model::{Category, FileIdentity};
use std::path::{Path, PathBuf};

pub use providers::{
    CargoTargetScanner, GradleScanner, NextCacheScanner, NodeModulesScanner, PyCacheScanner,
    TurboCacheScanner,
};
#[cfg(feature = "bench")]
pub use pruning::PrunedPaths;
pub use traversal::traverse_workspace;

pub trait Scanner: Send + Sync {
    fn category(&self) -> Category;
    fn matches(&self, name: &str) -> bool;
    fn confirm_context(&self, path: &Path) -> bool;
}

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
    pub identity: Option<FileIdentity>,
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
        Self::with_scanner(NodeModulesScanner)
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

    pub fn scanner_for(&self, category: &Category) -> Option<&dyn Scanner> {
        self.scanners
            .iter()
            .find(|scanner| &scanner.category() == category)
            .map(Box::as_ref)
    }

    /// Returns the category of the first provider that both matches the name and
    /// confirms the surrounding context. Traversal uses this instead of reaching
    /// into the scanner collection.
    pub fn classify(&self, name: &str, path: &Path) -> Option<Category> {
        self.scanners
            .iter()
            .find(|scanner| scanner.matches(name) && scanner.confirm_context(path))
            .map(|scanner| scanner.category())
    }

    pub fn categories(&self) -> impl Iterator<Item = Category> + '_ {
        self.scanners.iter().map(|scanner| scanner.category())
    }
}

impl Default for ProviderSet {
    fn default() -> Self {
        Self::new()
    }
}
