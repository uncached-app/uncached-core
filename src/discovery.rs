//! Discovery of caches that live outside any workspace.
//!
//! A workspace walk only finds what sits under a directory the user chose. The
//! largest caches on a developer's machine do not: Xcode's `DerivedData`, the
//! npm and pnpm stores, the Cargo registry and an editor's indexes all live in
//! the home directory, and none of them are reachable from a project folder.
//!
//! This module finds them by probing a curated table rather than by walking.
//! Walking a home directory would be slow, would read a great deal that is none
//! of the product's business, and would still have to decide what each hit is.
//! A table answers all three at once: every entry is a path we can name, with a
//! tool we can name, and a kind that says what deleting it costs.
//!
//! The table is deliberately narrow. An entry earns its place by being a proven
//! subpath of a tool's cache — never the tool's own root directory, which
//! usually mixes disposable state with configuration and history the user
//! cannot get back.

use crate::confidence::ConfidenceInput;
use crate::model::Category;
use crate::scanner::{ProviderSet, Scanner};
use std::path::{Path, PathBuf};

/// What deleting a location actually costs.
///
/// This is a different question from confidence. Confidence asks whether we
/// identified the thing correctly; kind asks what happens once it is gone.
/// Both feed the level, and only [`LocationKind::Artifact`] can reach `High`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationKind {
    /// Regenerable output. Deleting it costs one rebuild of one thing.
    Artifact,
    /// A registry, store or download cache shared by every project.
    ///
    /// Deleting it recovers space and makes every future install slower —
    /// including the installs that follow the cleanup this product just did.
    /// Emptying the pnpm store is the most self-defeating action a disk
    /// cleaner can take, so a shared cache is never offered as `High` and never
    /// enters automation.
    SharedCache,
    /// State belonging to a tool rather than to a build.
    ///
    /// Two different problems land here, and both mean the same thing to the
    /// user: look before you remove this.
    ///
    /// The first is content nothing regenerates, mixed in with content that
    /// does. `~/.m2/repository` holds crates fetched from a registry next to
    /// artifacts a local `mvn install` put there, which may be the only copy in
    /// existence. A JetBrains cache directory holds indexes beside Local
    /// History, the IDE's own record of edits to files that were never
    /// committed. Both look like caches from the outside.
    ///
    /// The second is removal that cannot be relied on to finish. Go marks its
    /// module cache read-only, so a permanent delete can fail partway and leave
    /// a broken cache behind — worse than not having tried.
    ///
    /// Reported so the user can see the size, always at `Review`.
    ToolState,
    /// Weight worth knowing about that this product will not remove.
    ///
    /// Some of the largest directories a developer owns are neither artifacts
    /// nor caches: a simulator device holds whatever was installed and
    /// configured inside it, and an Xcode archive holds the dSYMs that are the
    /// only way to symbolicate a crash from a release already in users' hands.
    /// Deleting either is a real decision with a real cost, and the tool that
    /// wrote them usually has a safer way to reclaim the space than removing
    /// the directory — `xcrun simctl delete unavailable` prunes exactly the
    /// devices macOS orphaned and leaves the rest alone.
    ///
    /// So the entry exists to report the size and name that operation. No
    /// provider vouches for it, which means cleanup refuses it at the same gate
    /// that refuses a misidentified path: the guarantee is structural, not a
    /// button that happens to be hidden.
    ReportOnly,
}

/// One entry of the table: a path we can name, under a tool we can name.
#[derive(Debug, Clone)]
pub struct KnownLocation {
    /// How the finding is named in the interface. The last path segment is
    /// rarely usable on its own — `_cacache`, `store` and `caches` say nothing
    /// about which tool wrote them.
    pub label: &'static str,
    /// Path relative to the home directory, always `/`-separated.
    pub relative: &'static str,
    pub category: Category,
    pub kind: LocationKind,
}

impl KnownLocation {
    pub fn resolve(&self, home: &Path) -> PathBuf {
        self.relative
            .split('/')
            .fold(home.to_path_buf(), |path, segment| path.join(segment))
    }
}

/// A location from the table that exists on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub path: PathBuf,
    pub label: &'static str,
    pub category: Category,
    pub kind: LocationKind,
}

/// The user's home directory, from the environment.
///
/// Read rather than resolved through the platform's user database: the product
/// only ever wants the home of whoever is running it, and an environment that
/// deliberately overrides `HOME` — a test, a sandbox — means it.
pub fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let raw = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let raw = std::env::var_os("HOME");

    let path = PathBuf::from(raw?);
    (path.is_absolute() && path.is_dir()).then_some(path)
}

/// Returns every table entry that exists, as a directory, under `home`.
///
/// Sizes are not measured here. Discovery answers what is there; measuring it
/// is the caller's decision, because the caller is the one that knows whether
/// it can afford to walk a 40 GB `DerivedData` right now.
pub fn probe_home(home: &Path) -> Vec<Discovery> {
    known_locations()
        .iter()
        .filter_map(|location| {
            let path = location.resolve(home);
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            // A symlink is followed by nobody here: the table names real
            // directories, and a link standing in for one is a fact about this
            // machine that we did not verify.
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return None;
            }
            // Reported resolved, like traversal reports its findings. A home
            // reached through a symlink would otherwise produce discoveries
            // that no ignore rule written against the real path can match, and
            // that cleanup canonicalizes into something else again.
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            Some(Discovery {
                path,
                label: location.label,
                category: location.category.clone(),
                kind: location.kind,
            })
        })
        .collect()
}

/// Signals for a discovered location.
///
/// A discovery skips the project reasoning entirely. There is no manifest above
/// `~/Library/Caches`, and there was never meant to be one: we know what the
/// directory is because we named it, not because we inferred it. Reusing
/// [`crate::confidence::collect_signals`] here would report "no project found
/// around it" for every single entry, which is true and useless.
pub fn signals_for(discovery: &Discovery) -> ConfidenceInput {
    ConfidenceInput {
        known_artifact: true,
        location_kind: Some(discovery.kind),
        ..ConfidenceInput::default()
    }
}

/// The table, per platform.
pub fn known_locations() -> &'static [KnownLocation] {
    PLATFORM_LOCATIONS
}

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
}

impl KnownLocationScanner {
    /// The provider set covering every catalogued location under `home`.
    ///
    /// One scanner per category, because a `ProviderSet` maps each category to
    /// exactly one provider.
    pub fn provider_set(home: PathBuf) -> ProviderSet {
        let mut providers = ProviderSet::new();
        for category in [
            Category::XcodeDerivedData,
            Category::PackageManagerCache,
            Category::AiToolCache,
            Category::EditorCache,
        ] {
            providers.add_scanner(Self {
                home: home.clone(),
                category,
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
    }

    fn confirm_context(&self, path: &Path) -> bool {
        self.entries()
            .any(|location| same_path(&location.resolve(&self.home), path))
    }
}

fn last_segment(relative: &str) -> &str {
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
        std::fs::canonicalize(catalogued),
        std::fs::canonicalize(candidate),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

macro_rules! locations {
    ($($label:literal => $relative:literal, $category:expr, $kind:expr;)*) => {
        &[$(KnownLocation {
            label: $label,
            relative: $relative,
            category: $category,
            kind: $kind,
        }),*]
    };
}

use Category::{AiToolCache, EditorCache, PackageManagerCache};
use LocationKind::{Artifact, SharedCache, ToolState};

// Only the macOS catalogue names these. Imported unconditionally they are dead
// on Linux and Windows, and a warning that is expected on two of three
// platforms is a warning nobody reads — which is how the next real one gets
// past. Everywhere else in this file refers to them fully qualified, so the
// tests are unaffected.
#[cfg(target_os = "macos")]
use Category::XcodeDerivedData;
#[cfg(target_os = "macos")]
use LocationKind::ReportOnly;

// `Archives` and `CoreSimulator/Devices` are catalogued as `ReportOnly`. They
// are usually the two heaviest directories Xcode owns, and neither is ours to
// remove: an archive holds the dSYMs that symbolicate crashes from a release
// already in users' hands, and a simulator device holds whatever was installed
// and configured inside it. Both name the operation that reclaims them safely.
// `CoreSimulator/Caches` next to them is an ordinary artifact.
#[cfg(target_os = "macos")]
static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "Xcode DerivedData" => "Library/Developer/Xcode/DerivedData", XcodeDerivedData, Artifact;
    "Simulator devices" => "Library/Developer/CoreSimulator/Devices", XcodeDerivedData, ReportOnly;
    "Xcode archives" => "Library/Developer/Xcode/Archives", XcodeDerivedData, ReportOnly;
    "Xcode iOS device support" => "Library/Developer/Xcode/iOS DeviceSupport", XcodeDerivedData, Artifact;
    "Xcode watchOS device support" => "Library/Developer/Xcode/watchOS DeviceSupport", XcodeDerivedData, Artifact;
    "Xcode tvOS device support" => "Library/Developer/Xcode/tvOS DeviceSupport", XcodeDerivedData, Artifact;
    "Simulator caches" => "Library/Developer/CoreSimulator/Caches", XcodeDerivedData, Artifact;
    "Xcode cache" => "Library/Caches/com.apple.dt.Xcode", XcodeDerivedData, Artifact;

    "npm cache" => ".npm/_cacache", PackageManagerCache, SharedCache;
    "pnpm store" => "Library/pnpm/store", PackageManagerCache, SharedCache;
    "pnpm cache" => "Library/Caches/pnpm", PackageManagerCache, SharedCache;
    "Yarn cache" => "Library/Caches/Yarn", PackageManagerCache, SharedCache;
    "Yarn Berry cache" => ".yarn/berry/cache", PackageManagerCache, SharedCache;
    "Bun cache" => ".bun/install/cache", PackageManagerCache, SharedCache;
    "Cargo registry" => ".cargo/registry", PackageManagerCache, SharedCache;
    "Gradle cache" => ".gradle/caches", PackageManagerCache, SharedCache;
    "Maven repository" => ".m2/repository", PackageManagerCache, ToolState;
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache;
    "Go module cache" => "go/pkg/mod", PackageManagerCache, ToolState;
    "Go build cache" => "Library/Caches/go-build", PackageManagerCache, SharedCache;
    "pip cache" => "Library/Caches/pip", PackageManagerCache, SharedCache;
    "CocoaPods cache" => "Library/Caches/CocoaPods", PackageManagerCache, SharedCache;
    "Homebrew downloads" => "Library/Caches/Homebrew", PackageManagerCache, SharedCache;
    "node-gyp headers" => "Library/Caches/node-gyp", PackageManagerCache, SharedCache;
    "TypeScript cache" => "Library/Caches/typescript", PackageManagerCache, SharedCache;
    "Deno cache" => "Library/Caches/deno", PackageManagerCache, SharedCache;
    "Puppeteer browsers" => ".cache/puppeteer", PackageManagerCache, SharedCache;
    "Playwright browsers" => "Library/Caches/ms-playwright", PackageManagerCache, SharedCache;

    "Claude Code shell snapshots" => ".claude/shell-snapshots", AiToolCache, ToolState;
    "Claude Code IDE locks" => ".claude/ide", AiToolCache, ToolState;
    "Claude Code session environment" => ".claude/session-env", AiToolCache, ToolState;
    "Codex logs" => ".codex/log", AiToolCache, ToolState;
    "opencode cache" => ".cache/opencode", AiToolCache, ToolState;
    "opencode logs" => ".local/share/opencode/log", AiToolCache, ToolState;
    "opencode tool output" => ".local/share/opencode/tool-output", AiToolCache, ToolState;

    "JetBrains caches" => "Library/Caches/JetBrains", EditorCache, ToolState;
    "VS Code cache" => "Library/Application Support/Code/Cache", EditorCache, Artifact;
    "VS Code cached data" => "Library/Application Support/Code/CachedData", EditorCache, Artifact;
    "VS Code GPU cache" => "Library/Application Support/Code/GPUCache", EditorCache, Artifact;
    "VS Code extension packages" => "Library/Application Support/Code/CachedExtensionVSIXs", EditorCache, Artifact;
};

#[cfg(target_os = "linux")]
static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "npm cache" => ".npm/_cacache", PackageManagerCache, SharedCache;
    "pnpm store" => ".local/share/pnpm/store", PackageManagerCache, SharedCache;
    "pnpm cache" => ".cache/pnpm", PackageManagerCache, SharedCache;
    "Yarn cache" => ".cache/yarn", PackageManagerCache, SharedCache;
    "Yarn Berry cache" => ".yarn/berry/cache", PackageManagerCache, SharedCache;
    "Bun cache" => ".bun/install/cache", PackageManagerCache, SharedCache;
    "Cargo registry" => ".cargo/registry", PackageManagerCache, SharedCache;
    "Gradle cache" => ".gradle/caches", PackageManagerCache, SharedCache;
    "Maven repository" => ".m2/repository", PackageManagerCache, ToolState;
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache;
    "Go module cache" => "go/pkg/mod", PackageManagerCache, ToolState;
    "Go build cache" => ".cache/go-build", PackageManagerCache, SharedCache;
    "pip cache" => ".cache/pip", PackageManagerCache, SharedCache;
    "node-gyp headers" => ".cache/node-gyp", PackageManagerCache, SharedCache;
    "Deno cache" => ".cache/deno", PackageManagerCache, SharedCache;
    "Puppeteer browsers" => ".cache/puppeteer", PackageManagerCache, SharedCache;
    "Playwright browsers" => ".cache/ms-playwright", PackageManagerCache, SharedCache;

    "Claude Code shell snapshots" => ".claude/shell-snapshots", AiToolCache, ToolState;
    "Claude Code IDE locks" => ".claude/ide", AiToolCache, ToolState;
    "Claude Code session environment" => ".claude/session-env", AiToolCache, ToolState;
    "Codex logs" => ".codex/log", AiToolCache, ToolState;
    "opencode cache" => ".cache/opencode", AiToolCache, ToolState;
    "opencode logs" => ".local/share/opencode/log", AiToolCache, ToolState;
    "opencode tool output" => ".local/share/opencode/tool-output", AiToolCache, ToolState;

    "JetBrains caches" => ".cache/JetBrains", EditorCache, ToolState;
    "VS Code cache" => ".config/Code/Cache", EditorCache, Artifact;
    "VS Code cached data" => ".config/Code/CachedData", EditorCache, Artifact;
    "VS Code GPU cache" => ".config/Code/GPUCache", EditorCache, Artifact;
};

#[cfg(windows)]
static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "npm cache" => "AppData/Local/npm-cache/_cacache", PackageManagerCache, SharedCache;
    "pnpm store" => "AppData/Local/pnpm/store", PackageManagerCache, SharedCache;
    "Yarn cache" => "AppData/Local/Yarn/Cache", PackageManagerCache, SharedCache;
    "Yarn Berry cache" => ".yarn/berry/cache", PackageManagerCache, SharedCache;
    "Bun cache" => ".bun/install/cache", PackageManagerCache, SharedCache;
    "Cargo registry" => ".cargo/registry", PackageManagerCache, SharedCache;
    "Gradle cache" => ".gradle/caches", PackageManagerCache, SharedCache;
    "Maven repository" => ".m2/repository", PackageManagerCache, ToolState;
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache;
    "Go module cache" => "go/pkg/mod", PackageManagerCache, ToolState;
    "Go build cache" => "AppData/Local/go-build", PackageManagerCache, SharedCache;
    "pip cache" => "AppData/Local/pip/Cache", PackageManagerCache, SharedCache;
    "node-gyp headers" => "AppData/Local/node-gyp/Cache", PackageManagerCache, SharedCache;
    "Deno cache" => "AppData/Local/deno", PackageManagerCache, SharedCache;
    "Puppeteer browsers" => ".cache/puppeteer", PackageManagerCache, SharedCache;
    "Playwright browsers" => "AppData/Local/ms-playwright", PackageManagerCache, SharedCache;

    "Claude Code shell snapshots" => ".claude/shell-snapshots", AiToolCache, ToolState;
    "Claude Code IDE locks" => ".claude/ide", AiToolCache, ToolState;
    "Claude Code session environment" => ".claude/session-env", AiToolCache, ToolState;
    "Codex logs" => ".codex/log", AiToolCache, ToolState;
    "opencode cache" => ".cache/opencode", AiToolCache, ToolState;

    "JetBrains caches" => "AppData/Local/JetBrains", EditorCache, ToolState;
    "VS Code cache" => "AppData/Roaming/Code/Cache", EditorCache, Artifact;
    "VS Code cached data" => "AppData/Roaming/Code/CachedData", EditorCache, Artifact;
    "VS Code GPU cache" => "AppData/Roaming/Code/GPUCache", EditorCache, Artifact;
};

#[cfg(test)]
mod tests {
    use super::{
        known_locations, probe_home, signals_for, Discovery, KnownLocation, KnownLocationScanner,
        LocationKind,
    };
    use crate::confidence::{assess, ConfidenceLevel};
    use crate::model::Category;
    use std::fs;
    use std::path::{Path, PathBuf};

    struct Home(PathBuf);

    impl Home {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("uncached-discovery-{name}"));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("home fixture should be created");
            Self(path)
        }

        fn create(&self, relative: &str) -> PathBuf {
            let path = relative
                .split('/')
                .fold(self.0.clone(), |path, segment| path.join(segment));
            fs::create_dir_all(&path).expect("location fixture should be created");
            path
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn first_of_kind(kind: LocationKind) -> &'static KnownLocation {
        known_locations()
            .iter()
            .find(|location| location.kind == kind)
            .expect("every platform table should carry each kind")
    }

    #[test]
    fn probing_reports_only_locations_that_exist() {
        let home = Home::new("exists");
        let location = first_of_kind(LocationKind::SharedCache);
        let expected = home.create(location.relative);

        let found = probe_home(home.path());

        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].path,
            expected.canonicalize().expect("fixture should resolve"),
            "discoveries are reported resolved, like traversal reports its own"
        );
        assert_eq!(found[0].label, location.label);
    }

    #[test]
    fn probing_an_untouched_home_finds_nothing() {
        let home = Home::new("empty");
        assert!(probe_home(home.path()).is_empty());
    }

    #[test]
    fn a_file_standing_where_a_directory_belongs_is_not_reported() {
        let home = Home::new("file");
        let location = first_of_kind(LocationKind::SharedCache);
        let path = location.resolve(home.path());
        fs::create_dir_all(path.parent().expect("location should have a parent"))
            .expect("parent should be created");
        fs::write(&path, b"not a cache").expect("file fixture should be written");

        assert!(probe_home(home.path()).is_empty());
    }

    #[test]
    fn a_shared_cache_never_reaches_high() {
        let discovery = Discovery {
            path: PathBuf::from("/home/user/.npm/_cacache"),
            label: "npm cache",
            category: Category::PackageManagerCache,
            kind: LocationKind::SharedCache,
        };

        assert_eq!(
            assess(signals_for(&discovery)).level,
            ConfidenceLevel::Medium
        );
    }

    #[test]
    fn tool_state_stays_at_review() {
        let discovery = Discovery {
            path: PathBuf::from("/home/user/.claude/shell-snapshots"),
            label: "Claude Code shell snapshots",
            category: Category::AiToolCache,
            kind: LocationKind::ToolState,
        };

        assert_eq!(
            assess(signals_for(&discovery)).level,
            ConfidenceLevel::Review
        );
    }

    #[test]
    fn a_named_artifact_location_reaches_high() {
        let discovery = Discovery {
            path: PathBuf::from("/home/user/Library/Developer/Xcode/DerivedData"),
            label: "Xcode DerivedData",
            category: Category::XcodeDerivedData,
            kind: LocationKind::Artifact,
        };

        assert_eq!(assess(signals_for(&discovery)).level, ConfidenceLevel::High);
    }

    #[test]
    fn no_entry_targets_a_tools_root_directory() {
        // The rule that keeps discovery from deleting configuration and
        // history: every entry names a subpath, never the tool's own folder.
        for location in known_locations() {
            let depth = location.relative.split('/').count();
            assert!(
                depth >= 2,
                "{} targets the root {}",
                location.label,
                location.relative
            );
        }
    }

    #[test]
    fn every_entry_is_relative_and_uniquely_named() {
        let mut seen = Vec::new();
        for location in known_locations() {
            assert!(
                !location.relative.starts_with('/') && !location.relative.contains('\\'),
                "{} is not a home-relative path",
                location.relative
            );
            assert!(
                !seen.contains(&location.relative),
                "{} appears twice",
                location.relative
            );
            seen.push(location.relative);
        }
    }

    /// Cleanup will not touch a path no provider vouches for, so every entry
    /// meant to be cleanable has to be recognized by the provider set built
    /// from the catalogue. Without this the targets would be visible and
    /// undeletable — reported as removable, and then refused.
    #[test]
    fn the_provider_set_vouches_for_every_cleanable_location() {
        let home = Home::new("providers");
        let providers = KnownLocationScanner::provider_set(home.path().to_path_buf());

        for location in known_locations() {
            if location.kind == LocationKind::ReportOnly {
                continue;
            }
            let path = home.create(location.relative);
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("fixture name should be valid UTF-8");
            let scanner = providers
                .scanner_for(&location.category)
                .unwrap_or_else(|| panic!("{} has no provider", location.label));

            assert!(scanner.matches(name), "{} name rejected", location.label);
            assert!(
                scanner.confirm_context(&path),
                "{} context rejected",
                location.label
            );
        }
    }

    /// The other half of the same rule, and the one that carries the promise: a
    /// `ReportOnly` entry is refused by the provider that owns its category, so
    /// cleanup cannot remove it through any path. The interface never has to
    /// remember to hide a button.
    #[test]
    fn no_provider_vouches_for_a_report_only_location() {
        let home = Home::new("report-only");
        let providers = KnownLocationScanner::provider_set(home.path().to_path_buf());
        let mut checked = 0;

        for location in known_locations() {
            if location.kind != LocationKind::ReportOnly {
                continue;
            }
            checked += 1;
            let path = home.create(location.relative);
            let Some(scanner) = providers.scanner_for(&location.category) else {
                continue;
            };
            assert!(
                !scanner.confirm_context(&path),
                "{} must never be confirmed for deletion",
                location.label
            );
        }

        // Guards the guard: a catalogue that lost its `ReportOnly` entries would
        // otherwise pass this test by checking nothing.
        assert!(
            checked > 0 || cfg!(not(target_os = "macos")),
            "the macOS catalogue should carry report-only entries"
        );
    }

    #[test]
    fn a_provider_refuses_a_path_that_is_not_in_the_catalogue() {
        let home = Home::new("decoy");
        let location = first_of_kind(LocationKind::SharedCache);
        let providers = KnownLocationScanner::provider_set(home.path().to_path_buf());
        let scanner = providers
            .scanner_for(&location.category)
            .expect("shared caches should have a provider");

        // The right basename in the wrong place: exactly the mistake the
        // catalogue exists to make impossible.
        let decoy = home.create(&format!(
            "elsewhere/{}",
            super::last_segment(location.relative)
        ));

        assert!(scanner.matches(super::last_segment(location.relative)));
        assert!(!scanner.confirm_context(&decoy));
    }

    /// These four were catalogued as ordinary caches on the strength of their
    /// names, and each one can destroy something nothing brings back. The names
    /// have not changed and neither has the temptation, so the ratings are
    /// pinned here rather than left to the next reader's judgement.
    #[test]
    fn the_locations_that_can_hold_originals_stay_at_review() {
        for relative in [
            // `mvn install` writes here; those artifacts exist nowhere else.
            ".m2/repository",
            // Local History lives under the cache directory. It is the record
            // of edits to files that were never committed.
            "Library/Caches/JetBrains",
            ".cache/JetBrains",
            "AppData/Local/JetBrains",
            // Read-only by design, so a permanent delete can fail partway.
            "go/pkg/mod",
        ] {
            let Some(location) = known_locations()
                .iter()
                .find(|location| location.relative == relative)
            else {
                continue; // Not catalogued on this platform.
            };
            assert_eq!(
                location.kind,
                LocationKind::ToolState,
                "{} is rated as if everything inside it were regenerable",
                location.label
            );
        }
    }

    #[test]
    fn ai_tool_entries_are_never_offered_as_disposable() {
        for location in known_locations() {
            if location.category == Category::AiToolCache {
                assert_eq!(
                    location.kind,
                    LocationKind::ToolState,
                    "{} claims more certainty than we have about AI tool state",
                    location.label
                );
            }
        }
    }
}
