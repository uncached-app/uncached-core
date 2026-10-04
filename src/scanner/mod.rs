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
/// Whether a provider needs a project beside a directory to classify it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Requires {
    /// A marker file or directory confirms the project.
    ProjectContext,
    /// The directory name alone identifies it.
    NameOnly,
}

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

    /// Every built-in provider.
    pub fn all() -> Self {
        Self::builtin(|_| true)
    }

    /// Returns only providers that require project context before classifying a
    /// directory. This is the conservative policy for callers where deleting a
    /// same-named directory is more dangerous than missing a cache.
    pub fn all_contextual() -> Self {
        Self::builtin(|requires| requires == Requires::ProjectContext)
    }

    /// The built-in providers in registration order, keeping those `keep`
    /// accepts. One list serves both sets above, so they cannot drift apart:
    /// the contextual set is this one minus the providers that classify by
    /// name alone.
    fn builtin(keep: impl Fn(Requires) -> bool) -> Self {
        use Requires::{NameOnly, ProjectContext};
        let fixed: [(Box<dyn Scanner>, Requires); 11] = [
            // React Native first: it claims paths inside `ios/` and
            // `android/` that Gradle and CocoaPods would otherwise claim by
            // their bare names, and the first provider to confirm wins.
            (
                Box::new(providers::ecosystems::react_native()),
                ProjectContext,
            ),
            (Box::new(NodeModulesScanner), ProjectContext),
            (Box::new(CargoTargetScanner), ProjectContext),
            (Box::new(NextCacheScanner), ProjectContext),
            (Box::new(TurboCacheScanner), ProjectContext),
            // Tool-generated caches whose name alone identifies them.
            (Box::new(PyCacheScanner), NameOnly),
            (Box::new(GradleScanner), NameOnly),
            (Box::new(XcodeDerivedDataScanner), ProjectContext),
            (Box::new(SwiftBuildScanner), ProjectContext),
            (Box::new(DotNetObjScanner), ProjectContext),
            (Box::new(DartToolScanner), ProjectContext),
        ];
        let mut set = Self::new();
        for (scanner, requires) in fixed {
            if keep(requires) {
                set.scanners.push(scanner);
            }
        }
        // Every project-type provider requires its ecosystem's marker beside
        // the directory, so these belong to both sets. `react_native` is listed
        // here as well, and `add_scanner` keeps the one registered first.
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

#[cfg(test)]
mod tests {
    use super::ProviderSet;
    use crate::model::Category;

    fn categories(set: &ProviderSet) -> Vec<Category> {
        set.categories().collect()
    }

    /// React Native claims paths inside `ios/` and `android/` that Gradle and
    /// CocoaPods would claim by bare name, and the first provider to confirm
    /// wins, so it has to be first in every set that has it.
    #[test]
    fn react_native_is_registered_before_the_providers_it_overlaps() {
        for set in [ProviderSet::all(), ProviderSet::all_contextual()] {
            assert_eq!(categories(&set).first(), Some(&Category::ReactNative));
        }
    }

    #[test]
    fn every_category_is_registered_once() {
        for set in [ProviderSet::all(), ProviderSet::all_contextual()] {
            let found = categories(&set);
            for (index, category) in found.iter().enumerate() {
                assert!(!found[..index].contains(category), "{category:?} twice");
            }
        }
    }

    /// The contextual set is the full set minus the providers that classify by
    /// name alone, in the same order — the property that keeping one list is
    /// meant to guarantee.
    #[test]
    fn the_contextual_set_is_the_full_set_without_the_name_only_providers() {
        let by_name_alone = [Category::PyCache, Category::Gradle];
        let expected: Vec<_> = categories(&ProviderSet::all())
            .into_iter()
            .filter(|category| !by_name_alone.contains(category))
            .collect();

        assert_eq!(categories(&ProviderSet::all_contextual()), expected);
        assert_eq!(
            ProviderSet::all().len(),
            expected.len() + by_name_alone.len()
        );
    }
}
