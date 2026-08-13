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
    /// Regenerable in principle, but the boundary between scratch and real user
    /// data inside these directories is a judgement call we have not earned
    /// yet. Reported so the user can see the size, always at `Review`.
    ToolState,
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
            (metadata.is_dir() && !metadata.file_type().is_symlink()).then_some(Discovery {
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

    fn entries(&self) -> impl Iterator<Item = &'static KnownLocation> + '_ {
        known_locations()
            .iter()
            .filter(move |location| location.category == self.category)
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

use Category::{AiToolCache, EditorCache, PackageManagerCache, XcodeDerivedData};
use LocationKind::{Artifact, SharedCache, ToolState};

// Two Xcode directories are deliberately absent. `Archives` holds shipped
// builds and their dSYMs, which are the only way to symbolicate a crash report
// from a release already in users' hands; nothing regenerates those. And
// `CoreSimulator/Devices` holds simulator state, including whatever a developer
// installed and configured there. `CoreSimulator/Caches` next to it is fine.
#[cfg(target_os = "macos")]
static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "Xcode DerivedData" => "Library/Developer/Xcode/DerivedData", XcodeDerivedData, Artifact;
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
    "Maven repository" => ".m2/repository", PackageManagerCache, SharedCache;
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache;
    "Go module cache" => "go/pkg/mod", PackageManagerCache, SharedCache;
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

    "JetBrains caches" => "Library/Caches/JetBrains", EditorCache, Artifact;
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
    "Maven repository" => ".m2/repository", PackageManagerCache, SharedCache;
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache;
    "Go module cache" => "go/pkg/mod", PackageManagerCache, SharedCache;
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

    "JetBrains caches" => ".cache/JetBrains", EditorCache, Artifact;
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
    "Maven repository" => ".m2/repository", PackageManagerCache, SharedCache;
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache;
    "Go module cache" => "go/pkg/mod", PackageManagerCache, SharedCache;
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

    "JetBrains caches" => "AppData/Local/JetBrains", EditorCache, Artifact;
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
        assert_eq!(found[0].path, expected);
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

    /// Cleanup will not touch a path no provider vouches for, so every entry in
    /// the catalogue has to be recognized by the provider set built from it.
    /// Without this the discovered targets would be visible and undeletable.
    #[test]
    fn the_provider_set_vouches_for_every_catalogued_location() {
        let home = Home::new("providers");
        let providers = KnownLocationScanner::provider_set(home.path().to_path_buf());

        for location in known_locations() {
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
