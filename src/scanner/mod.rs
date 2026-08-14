pub(crate) mod providers;
mod pruning;
mod traversal;

use crate::model::{Category, FileIdentity};
use std::path::{Path, PathBuf};

pub use providers::{
    ecosystems, CargoTargetScanner, DartToolScanner, DotNetObjScanner, GradleScanner,
    NextCacheScanner, NodeModulesScanner, ProjectMarker, ProjectScanner, PyCacheScanner,
    SwiftBuildScanner, TurboCacheScanner, XcodeDerivedDataScanner,
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

/// What a scan looks for, and what it must leave alone.
///
/// The ignore rules live here rather than beside the traversal because they
/// have to reach two places that are far apart in time: the walk, which never
/// descends into an excluded directory, and the cleanup, which refuses one even
/// if it was already found. Carrying them with the providers makes that
/// structural — every function that can authorize a deletion already takes a
/// `ProviderSet`, so none of them can be called without the rules in hand.
pub struct ProviderSet {
    scanners: Vec<Box<dyn Scanner>>,
    ignore_rules: crate::ignore_rules::IgnoreRules,
}

impl ProviderSet {
    pub fn new() -> Self {
        Self {
            scanners: Vec::new(),
            ignore_rules: crate::ignore_rules::IgnoreRules::default(),
        }
    }

    pub fn with_scanner(scanner: impl Scanner + 'static) -> Self {
        Self {
            scanners: vec![Box::new(scanner)],
            ignore_rules: crate::ignore_rules::IgnoreRules::default(),
        }
    }

    /// Attaches the user's exclusions. Without this a set protects nothing,
    /// which is the right default for a caller that has no rules to apply.
    pub fn with_ignore_rules(mut self, rules: crate::ignore_rules::IgnoreRules) -> Self {
        self.ignore_rules = rules;
        self
    }

    /// Whether the user has told us to leave this path alone.
    pub fn excludes(&self, path: &Path) -> bool {
        self.ignore_rules.excludes(path)
    }

    pub fn ignore_rules(&self) -> &crate::ignore_rules::IgnoreRules {
        &self.ignore_rules
    }

    /// Adds a provider unless its category is already registered.
    ///
    /// Returns `true` when the provider was added and `false` when the existing
    /// provider was kept. Category uniqueness makes lookup and classification
    /// independent of insertion order.
    pub fn add_scanner(&mut self, scanner: impl Scanner + 'static) -> bool {
        let category = scanner.category();
        if self
            .scanners
            .iter()
            .any(|registered| registered.category() == category)
        {
            return false;
        }
        self.scanners.push(Box::new(scanner));
        true
    }

    pub fn free() -> Self {
        Self::with_scanner(NodeModulesScanner)
    }

    pub fn all() -> Self {
        let mut set = Self {
            scanners: vec![
                // React Native first: it claims paths inside `ios/` and
                // `android/` that Gradle and CocoaPods would otherwise claim by
                // their bare names, and the first provider to confirm wins.
                Box::new(providers::ecosystems::react_native()),
                Box::new(NodeModulesScanner),
                Box::new(CargoTargetScanner),
                Box::new(NextCacheScanner),
                Box::new(TurboCacheScanner),
                Box::new(PyCacheScanner),
                Box::new(GradleScanner),
                Box::new(XcodeDerivedDataScanner),
                Box::new(SwiftBuildScanner),
                Box::new(DotNetObjScanner),
                Box::new(DartToolScanner),
            ],
            ignore_rules: crate::ignore_rules::IgnoreRules::default(),
        };
        for scanner in providers::ecosystems::all() {
            set.add_scanner(scanner);
        }
        set
    }

    /// Returns only providers that require project context before classifying a
    /// directory. This is the conservative policy for callers where deleting a
    /// same-named directory is more dangerous than missing a cache.
    pub fn all_contextual() -> Self {
        let mut set = Self {
            scanners: vec![
                Box::new(providers::ecosystems::react_native()),
                Box::new(NodeModulesScanner),
                Box::new(CargoTargetScanner),
                Box::new(NextCacheScanner),
                Box::new(TurboCacheScanner),
                Box::new(XcodeDerivedDataScanner),
                Box::new(SwiftBuildScanner),
                Box::new(DotNetObjScanner),
                Box::new(DartToolScanner),
            ],
            ignore_rules: crate::ignore_rules::IgnoreRules::default(),
        };
        // Every project-type provider requires its ecosystem's marker beside
        // the directory, which is exactly the property this set selects for.
        for scanner in providers::ecosystems::all() {
            set.add_scanner(scanner);
        }
        set
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
