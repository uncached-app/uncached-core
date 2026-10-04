use super::catalogue::{known_locations, KnownLocation};
use super::probe::{probe_home, probe_temp};
use crate::model::{Category, LocationKind};
use crate::scanner::{ProviderSet, Scanner};
use std::path::{Path, PathBuf};

/// A provider that recognizes one category of catalogued location.
///
/// Discovery does not get its own way to delete things. Cleanup asks a provider
/// to vouch for a target before touching it, and that check is the reason a
/// misidentified directory cannot be removed, so a discovered cache has to be
/// vouched for by a provider like anything else. This is that provider: it
/// confirms a path by finding it in the catalogue, which is a stricter test
/// than any name match — the whole path must be one we named in advance.
pub struct KnownLocationScanner {
    home: PathBuf,
    category: Category,
    /// Scratch outside the home directory, resolved once when the set is built.
    ///
    /// The catalogue is a list of paths named in advance, and that is what
    /// makes vouching stricter than a name match. A temp directory carries the
    /// user id in its name, so it cannot be written down in advance — but it is
    /// still *named in advance* in every sense that matters: the prefix, the
    /// owner and the root are ours, and [`probe_temp`] is the only thing that
    /// can put a path in here.
    temp: Vec<PathBuf>,
    /// Catalogued entries as they resolved on this machine.
    ///
    /// A moved cache is still the entry the table names, but its directory can
    /// be called anything: `~/Library/pnpm/store` may lead to `pnpm-cache` on
    /// another volume, and a name match against `store` would refuse it. The
    /// path is still named in advance — [`probe_home`] reaches it only by
    /// following a catalogued entry — so the name it landed under is vouched
    /// for alongside the table's own.
    resolved: Vec<PathBuf>,
}

impl KnownLocationScanner {
    /// The provider set covering every catalogued location under `home`.
    ///
    /// One scanner per category, because a `ProviderSet` maps each category to
    /// exactly one provider.
    pub fn provider_set(home: PathBuf) -> ProviderSet {
        let mut providers = ProviderSet::new();
        let scratch = probe_temp(&home);
        let catalogued = probe_home(&home);
        for category in [
            Category::XcodeDerivedData,
            Category::PackageManagerCache,
            Category::AiToolCache,
            Category::EditorCache,
            Category::UnityLibrary,
            Category::GodotCache,
            Category::AndroidSdk,
        ] {
            let temp = scratch
                .iter()
                .filter(|discovery| {
                    discovery.category == category && discovery.kind != LocationKind::ReportOnly
                })
                .map(|discovery| discovery.path.clone())
                .collect();
            let resolved = catalogued
                .iter()
                .filter(|discovery| {
                    discovery.category == category && discovery.kind != LocationKind::ReportOnly
                })
                .map(|discovery| discovery.path.clone())
                .collect();
            providers.add_scanner(Self {
                home: home.clone(),
                category,
                temp,
                resolved,
            });
        }
        providers
    }

    /// The entries this provider will vouch for.
    ///
    /// `ReportOnly` is filtered out here rather than anywhere nearer the UI.
    /// Cleanup asks a provider to confirm a path before touching it, so an
    /// entry no provider claims cannot be deleted through any code path — the
    /// same refusal a misidentified directory gets. Hiding the button would
    /// have been a decision the interface could forget to make.
    fn entries(&self) -> impl Iterator<Item = &'static KnownLocation> + '_ {
        known_locations().iter().filter(move |location| {
            location.category == self.category && location.kind != LocationKind::ReportOnly
        })
    }
}

impl Scanner for KnownLocationScanner {
    fn category(&self) -> Category {
        self.category.clone()
    }

    fn matches(&self, name: &str) -> bool {
        self.entries()
            .any(|location| last_segment(location.relative) == name)
            || self
                .temp
                .iter()
                .chain(self.resolved.iter())
                .any(|path| path.file_name().is_some_and(|segment| segment == name))
    }

    fn confirm_context(&self, path: &Path) -> bool {
        self.entries()
            .any(|location| same_path(&location.resolve(&self.home), path))
            || self
                .temp
                .iter()
                .chain(self.resolved.iter())
                .any(|known| same_path(known, path))
    }
}

pub(super) fn last_segment(relative: &str) -> &str {
    relative.rsplit('/').next().unwrap_or(relative)
}

/// Cleanup canonicalizes the target before asking for confirmation, while the
/// catalogue is written in plain home-relative terms. Comparing the raw paths
/// alone would reject every entry on a machine where the home directory is
/// reached through a symlink.
fn same_path(catalogued: &Path, candidate: &Path) -> bool {
    if catalogued == candidate {
        return true;
    }
    match (
        crate::filesystem::canonical(catalogued),
        crate::filesystem::canonical(candidate),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
