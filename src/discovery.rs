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
    /// The executables that own this directory, by the name a process list
    /// reports them under.
    ///
    /// A cache is only disposable while nothing is using it. `.claude/ide`
    /// holds the lock files of *live* sessions; a package manager's store is
    /// being written to during an install. Removing either mid-flight does not
    /// free space, it breaks the thing that was running — and the tool cannot
    /// tell that from a corrupt cache afterwards.
    ///
    /// Empty means nothing here is worth checking for: a directory no running
    /// program holds open, or one whose owner cannot be named unambiguously.
    /// It is never a claim that removal is safe, only that this particular
    /// question has no answer to give.
    pub owner: &'static [&'static str],
}

impl KnownLocation {
    /// See [`Discovery::reclaim_with`].
    pub fn reclaim_with(&self) -> Option<&'static str> {
        match self.relative {
            "Library/Developer/CoreSimulator/Devices" => Some("xcrun simctl delete unavailable"),
            ".android/avd" => Some("avdmanager delete avd -n <name>"),
            "Library/Containers/com.docker.docker/Data/vms/0/data"
            | ".docker/desktop/vms/0/data"
            | "AppData/Local/Docker/wsl/data" => Some("docker system prune"),
            _ => None,
        }
    }

    pub fn resolve(&self, home: &Path) -> PathBuf {
        self.relative
            .split('/')
            .fold(home.to_path_buf(), |path, segment| path.join(segment))
    }
}

/// Scratch a tool keeps outside the home directory.
///
/// The catalogue above is written relative to the home directory, because that
/// is where a developer's caches live. Not all of them: a coding agent writes
/// its per-session working files under the system temp directory, where the
/// name carries the user id rather than being a fixed string — and on this
/// machine that tree was seven gigabytes, larger than every catalogued cache
/// put together and invisible to every screen this product has.
///
/// Probed rather than walked, on the same principle as the table. `/tmp` holds
/// live sockets and lock files belonging to every running program on the
/// machine, and reading it for anything we cannot name would be the walk this
/// module exists to avoid. Only directories this names, owned by the user
/// running us, are ever reported.
#[cfg(unix)]
#[derive(Debug, Clone)]
struct TempLocation {
    label: &'static str,
    /// Matched against the directory name up to the user id.
    prefix: &'static str,
    category: Category,
    kind: LocationKind,
    owner: &'static [&'static str],
}

/// The temp roots worth probing, in the order they are tried.
///
/// `/private/tmp` is where `/tmp` actually resolves on macOS, and the agent
/// writes the resolved form. Both are listed so a machine that reports either
/// is covered, and the canonicalisation in [`probe_temp`] keeps one answer.
#[cfg(unix)]
const TEMP_ROOTS: &[&str] = &["/tmp", "/private/tmp"];

#[cfg(unix)]
static TEMP_LOCATIONS: &[TempLocation] = &[TempLocation {
    label: "Claude Code session scratch",
    prefix: "claude-",
    category: Category::AiToolCache,
    kind: LocationKind::ToolState,
    owner: &["claude"],
}];

/// The scratch directories that exist on this machine, for the user running us.
///
/// `home` is what decides who that is: it is owned by the account this process
/// belongs to, so its owner id is ours without asking the platform for it. A
/// directory in `/tmp` belonging to somebody else is another account's session
/// and never ours to report, let alone remove.
#[cfg(unix)]
pub fn probe_temp(home: &Path) -> Vec<Discovery> {
    use std::os::unix::fs::MetadataExt;

    let Ok(us) = std::fs::metadata(home).map(|metadata| metadata.uid()) else {
        return Vec::new();
    };
    let mut found: Vec<Discovery> = Vec::new();

    for root in TEMP_ROOTS {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Some(location) = TEMP_LOCATIONS
                .iter()
                .find(|location| name.starts_with(location.prefix))
            else {
                continue;
            };
            if !is_session_scratch(&name, location.prefix) {
                continue;
            }
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.uid() != us {
                continue;
            }
            let path = crate::filesystem::canonical(entry.path()).unwrap_or_else(|_| entry.path());
            // `/tmp` and `/private/tmp` are the same tree on macOS, so the two
            // roots would otherwise report one directory twice.
            if found.iter().any(|discovery| discovery.path == path) {
                continue;
            }
            // Canonical, and the same tree the path was resolved into:
            // `/tmp` is a symlink to `/private/tmp` on macOS, and a root that
            // still said `/tmp` would not contain the path that says
            // `/private/tmp`.
            let root = crate::filesystem::canonical(Path::new(root))
                .unwrap_or_else(|_| PathBuf::from(root));
            found.push(Discovery {
                path,
                label: location.label,
                category: location.category.clone(),
                kind: location.kind,
                owner: location.owner,
                root,
                reclaim_with: None,
            });
        }
    }
    found
}

/// Whether a directory name is `<prefix><user id>` and nothing else.
///
/// The prefix alone would claim `claude-config`, which is somebody's settings
/// rather than a session's scratch.
#[cfg(unix)]
fn is_session_scratch(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
    })
}

#[cfg(not(unix))]
pub fn probe_temp(_home: &Path) -> Vec<Discovery> {
    // The agent's temp layout on Windows has not been confirmed, and a guess
    // here is a guess about a directory this product would offer to delete.
    Vec::new()
}

/// A machine-wide directory, named by absolute path.
///
/// Always [`LocationKind::ReportOnly`]: the type has no field that could say
/// otherwise. These are root-owned, and only the owning tool removes them.
#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct SystemLocation {
    label: &'static str,
    path: &'static str,
    category: Category,
    owner: &'static [&'static str],
    /// See [`Discovery::reclaim_with`].
    reclaim_with: &'static str,
}

// `CoreSimulator/Volumes` is deliberately absent: it is where the runtime images
// below are mounted, and sizing the mount counts their uncompressed contents.
#[cfg(target_os = "macos")]
static SYSTEM_LOCATIONS: &[SystemLocation] = &[SystemLocation {
    label: "Simulator runtimes",
    path: "/System/Library/AssetsV2/com_apple_MobileAsset_iOSSimulatorRuntime",
    category: Category::XcodeDerivedData,
    owner: &["Xcode", "Simulator"],
    reclaim_with: "xcrun simctl runtime delete --notUsedSinceDays 30",
}];

#[cfg(not(target_os = "macos"))]
static SYSTEM_LOCATIONS: &[SystemLocation] = &[];

/// Machine-wide locations that exist, every one of them report-only.
pub fn probe_system() -> Vec<Discovery> {
    probe_absolute(SYSTEM_LOCATIONS)
}

/// Links are not followed: a system path redirected elsewhere is not the
/// directory the table names.
fn probe_absolute(locations: &[SystemLocation]) -> Vec<Discovery> {
    locations
        .iter()
        .filter_map(|location| {
            let path = Path::new(location.path);
            let metadata = std::fs::symlink_metadata(path).ok()?;
            if !metadata.is_dir() || is_redirect(&metadata) {
                return None;
            }
            let path = crate::filesystem::canonical(path).ok()?;
            let root = path.parent()?.to_path_buf();
            Some(Discovery {
                path,
                label: location.label,
                category: location.category.clone(),
                kind: LocationKind::ReportOnly,
                owner: location.owner,
                root,
                reclaim_with: Some(location.reclaim_with),
            })
        })
        .collect()
}

/// A location from the table that exists on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub path: PathBuf,
    pub label: &'static str,
    pub category: Category,
    pub kind: LocationKind,
    /// See [`KnownLocation::owner`].
    pub owner: &'static [&'static str],
    /// The directory this was found under.
    ///
    /// Cleanup refuses anything outside the root its target was registered
    /// against — the rule that keeps a workspace scan from reaching past the
    /// folders the user chose. Catalogued locations used to be registered
    /// against the home directory because that is where all of them were.
    /// Session scratch is not: it lives under `/tmp`, so a target that named
    /// home as its root was one cleanup would refuse to remove, having offered
    /// it. The base travels with the discovery instead of being assumed.
    pub root: PathBuf,
    /// Shell command, run by the user, that reclaims a `ReportOnly` location
    /// through the tool that owns it. `None` when that tool has no command.
    pub reclaim_with: Option<&'static str>,
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

/// Whether a catalogued entry stands in for a directory somewhere else.
///
/// On Windows a relocated cache is usually a junction — `mklink /J` needs no
/// administrator, `mklink /D` does — and a junction carries
/// `IO_REPARSE_TAG_MOUNT_POINT` rather than `IO_REPARSE_TAG_SYMLINK`. Reading
/// the attribute covers every reparse tag without depending on which of them
/// the standard library chooses to call a symlink.
fn is_redirect(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

/// Returns every table entry that exists, as a directory, under `home`.
///
/// Sizes are not measured here. Discovery answers what is there; measuring it
/// is the caller's decision, because the caller is the one that knows whether
/// it can afford to walk a 40 GB `DerivedData` right now.
///
/// A moved cache is reached through the link left behind. Relocating a store
/// onto a second volume is ordinary on a machine whose boot disk is the small
/// one, and the entries that get moved are the largest ones the catalogue
/// names — so skipping them hid exactly the directories worth finding, and hid
/// them silently. The redirection is resolved and the discovery carries the
/// base it actually landed in, the same way session scratch under `/tmp` does.
pub fn probe_home(home: &Path) -> Vec<Discovery> {
    // Resolved once, for the same reason each path below is: a home reached
    // through a symlink would not contain the canonical paths found under it,
    // and containment is what cleanup checks before it removes anything.
    let root = crate::filesystem::canonical(home).unwrap_or_else(|_| home.to_path_buf());
    known_locations()
        .iter()
        .filter_map(|location| {
            let path = location.resolve(home);
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            let redirected = is_redirect(&metadata);
            // A redirection's own metadata describes the link, so what it
            // stands for is only known once resolved. Anything that does not
            // resolve to a directory — dangling, or pointing at a file — is
            // not the catalogued location and is left alone.
            if !redirected && !metadata.is_dir() {
                return None;
            }
            // Reported resolved, like traversal reports its findings. A home
            // reached through a symlink would otherwise produce discoveries
            // that no ignore rule written against the real path can match, and
            // that cleanup canonicalizes into something else again.
            let path = crate::filesystem::canonical(&path).ok()?;
            if redirected && !std::fs::metadata(&path).is_ok_and(|target| target.is_dir()) {
                return None;
            }
            // `is_within` wants a strict ancestor, so the base is the parent
            // rather than the directory itself. Only a redirection that leaves
            // home needs one: staying inside keeps the narrower base, and a
            // link landing at a filesystem root has no parent to register
            // against and is refused rather than registered against nothing.
            let discovery_root = if crate::filesystem::is_within(&root, &path) {
                root.clone()
            } else {
                path.parent()?.to_path_buf()
            };
            Some(Discovery {
                path,
                label: location.label,
                category: location.category.clone(),
                kind: location.kind,
                owner: location.owner,
                root: discovery_root,
                reclaim_with: location.reclaim_with(),
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
        crate::filesystem::canonical(catalogued),
        crate::filesystem::canonical(candidate),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

macro_rules! locations {
    ($($label:literal => $relative:literal, $category:expr, $kind:expr, $owner:expr;)*) => {
        &[$(KnownLocation {
            label: $label,
            relative: $relative,
            category: $category,
            kind: $kind,
            owner: $owner,
        }),*]
    };
}

use Category::{
    AiToolCache, AndroidSdk, Docker, EditorCache, GodotCache, PackageManagerCache, UnityLibrary,
};
use LocationKind::{Artifact, ReportOnly, SharedCache, ToolState};

// Only the macOS catalogue names these. Imported unconditionally they are dead
// on Linux and Windows, and a warning that is expected on two of three
// platforms is a warning nobody reads — which is how the next real one gets
// past. Everywhere else in this file refers to them fully qualified, so the
// tests are unaffected.
#[cfg(target_os = "macos")]
use Category::XcodeDerivedData;

// `Archives` and `CoreSimulator/Devices` are catalogued as `ReportOnly`. They
// are usually the two heaviest directories Xcode owns, and neither is ours to
// remove: an archive holds the dSYMs that symbolicate crashes from a release
// already in users' hands, and a simulator device holds whatever was installed
// and configured inside it. Both name the operation that reclaims them safely.
// `CoreSimulator/Caches` next to them is an ordinary artifact.
#[cfg(target_os = "macos")]
static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "Xcode DerivedData" => "Library/Developer/Xcode/DerivedData", XcodeDerivedData, Artifact, &["Xcode"];
    "Simulator devices" => "Library/Developer/CoreSimulator/Devices", XcodeDerivedData, ReportOnly, &["Xcode", "Simulator"];
    "Xcode archives" => "Library/Developer/Xcode/Archives", XcodeDerivedData, ReportOnly, &["Xcode"];
    "Xcode iOS device support" => "Library/Developer/Xcode/iOS DeviceSupport", XcodeDerivedData, Artifact, &["Xcode"];
    "Xcode watchOS device support" => "Library/Developer/Xcode/watchOS DeviceSupport", XcodeDerivedData, Artifact, &["Xcode"];
    "Xcode tvOS device support" => "Library/Developer/Xcode/tvOS DeviceSupport", XcodeDerivedData, Artifact, &["Xcode"];
    "Simulator caches" => "Library/Developer/CoreSimulator/Caches", XcodeDerivedData, Artifact, &["Xcode", "Simulator"];
    "Xcode cache" => "Library/Caches/com.apple.dt.Xcode", XcodeDerivedData, Artifact, &["Xcode"];

    "npm cache" => ".npm/_cacache", PackageManagerCache, SharedCache, &[];
    "pnpm store" => "Library/pnpm/store", PackageManagerCache, SharedCache, &[];
    "pnpm cache" => "Library/Caches/pnpm", PackageManagerCache, SharedCache, &[];
    "Yarn cache" => "Library/Caches/Yarn", PackageManagerCache, SharedCache, &[];
    "Yarn Berry cache" => ".yarn/berry/cache", PackageManagerCache, SharedCache, &[];
    "Bun cache" => ".bun/install/cache", PackageManagerCache, SharedCache, &["bun"];
    "Cargo registry" => ".cargo/registry", PackageManagerCache, SharedCache, &["cargo"];
    "Gradle cache" => ".gradle/caches", PackageManagerCache, SharedCache, &[];
    "Maven repository" => ".m2/repository", PackageManagerCache, ToolState, &[];
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache, &[];
    "Go module cache" => "go/pkg/mod", PackageManagerCache, ToolState, &["go"];
    "Go build cache" => "Library/Caches/go-build", PackageManagerCache, SharedCache, &["go"];
    "pip cache" => "Library/Caches/pip", PackageManagerCache, SharedCache, &[];
    "CocoaPods cache" => "Library/Caches/CocoaPods", PackageManagerCache, SharedCache, &[];
    "Homebrew downloads" => "Library/Caches/Homebrew", PackageManagerCache, SharedCache, &[];
    "node-gyp headers" => "Library/Caches/node-gyp", PackageManagerCache, SharedCache, &[];
    "TypeScript cache" => "Library/Caches/typescript", PackageManagerCache, SharedCache, &[];
    "Deno cache" => "Library/Caches/deno", PackageManagerCache, SharedCache, &["deno"];
    "Puppeteer browsers" => ".cache/puppeteer", PackageManagerCache, SharedCache, &[];
    "Playwright browsers" => "Library/Caches/ms-playwright", PackageManagerCache, SharedCache, &[];
    "uv cache" => ".cache/uv", PackageManagerCache, SharedCache, &["uv", "uvx"];

    "Unity package cache" => "Library/Caches/Unity/upm", UnityLibrary, SharedCache, &["Unity"];
    "Unity Asset Store packages" => "Library/Unity/Asset Store-5.x", UnityLibrary, ToolState, &["Unity"];
    "Unity build cache" => "Library/Unity/cache/bee", UnityLibrary, ToolState, &["Unity"];
    "Godot editor cache" => "Library/Caches/Godot", GodotCache, Artifact, &["Godot"];
    "Godot export templates" => "Library/Application Support/Godot/export_templates", GodotCache, SharedCache, &["Godot"];

    // Android's SDK downloads one copy of a system image for every project on
    // the machine; an emulator built from one holds whatever was installed
    // inside it, which is why only the first is cleanable.
    "Android system images" => "Library/Android/sdk/system-images", AndroidSdk, SharedCache, &["emulator", "qemu-system-aarch64", "studio"];
    "Android emulators" => ".android/avd", AndroidSdk, ReportOnly, &["emulator", "qemu-system-aarch64"];

    // One file holds every image, volume and container, including images built
    // here and pushed nowhere. Docker's own prune is the only safe way in.
    "Docker disk image" => "Library/Containers/com.docker.docker/Data/vms/0/data", Docker, ReportOnly, &["com.docker.backend", "Docker Desktop", "docker"];

    "Ollama models" => ".ollama/models", AiToolCache, ToolState, &["ollama"];

    "nvm download cache" => ".nvm/.cache", PackageManagerCache, SharedCache, &[];
    "Node versions (nvm)" => ".nvm/versions/node", PackageManagerCache, ToolState, &["node"];
    "pyenv build cache" => ".pyenv/cache", PackageManagerCache, SharedCache, &[];
    "Python versions (pyenv)" => ".pyenv/versions", PackageManagerCache, ToolState, &["python", "python3"];

    "Claude Code shell snapshots" => ".claude/shell-snapshots", AiToolCache, ToolState, &["claude"];
    "Claude Code IDE locks" => ".claude/ide", AiToolCache, ToolState, &["claude"];
    "Claude Code session environment" => ".claude/session-env", AiToolCache, ToolState, &["claude"];
    "Claude app cache" => "Library/Application Support/Claude/Cache", AiToolCache, ToolState, &["Claude"];
    "Claude app code cache" => "Library/Application Support/Claude/Code Cache", AiToolCache, ToolState, &["Claude"];
    "Codex logs" => ".codex/log", AiToolCache, ToolState, &["codex"];
    "opencode cache" => ".cache/opencode", AiToolCache, ToolState, &["opencode"];
    "opencode logs" => ".local/share/opencode/log", AiToolCache, ToolState, &["opencode"];
    "opencode tool output" => ".local/share/opencode/tool-output", AiToolCache, ToolState, &["opencode"];

    "JetBrains caches" => "Library/Caches/JetBrains", EditorCache, ToolState, &["idea", "pycharm", "webstorm", "goland", "clion", "phpstorm", "rubymine", "rustrover", "datagrip"];
    "VS Code cache" => "Library/Application Support/Code/Cache", EditorCache, Artifact, &["Code"];
    "VS Code cached data" => "Library/Application Support/Code/CachedData", EditorCache, Artifact, &["Code"];
    "VS Code GPU cache" => "Library/Application Support/Code/GPUCache", EditorCache, Artifact, &["Code"];
    "VS Code extension packages" => "Library/Application Support/Code/CachedExtensionVSIXs", EditorCache, Artifact, &["Code"];
};

#[cfg(target_os = "linux")]
static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "npm cache" => ".npm/_cacache", PackageManagerCache, SharedCache, &[];
    "pnpm store" => ".local/share/pnpm/store", PackageManagerCache, SharedCache, &[];
    "pnpm cache" => ".cache/pnpm", PackageManagerCache, SharedCache, &[];
    "Yarn cache" => ".cache/yarn", PackageManagerCache, SharedCache, &[];
    "Yarn Berry cache" => ".yarn/berry/cache", PackageManagerCache, SharedCache, &[];
    "Bun cache" => ".bun/install/cache", PackageManagerCache, SharedCache, &["bun"];
    "Cargo registry" => ".cargo/registry", PackageManagerCache, SharedCache, &["cargo"];
    "Gradle cache" => ".gradle/caches", PackageManagerCache, SharedCache, &[];
    "Maven repository" => ".m2/repository", PackageManagerCache, ToolState, &[];
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache, &[];
    "Go module cache" => "go/pkg/mod", PackageManagerCache, ToolState, &["go"];
    "Go build cache" => ".cache/go-build", PackageManagerCache, SharedCache, &["go"];
    "pip cache" => ".cache/pip", PackageManagerCache, SharedCache, &[];
    "node-gyp headers" => ".cache/node-gyp", PackageManagerCache, SharedCache, &[];
    "Deno cache" => ".cache/deno", PackageManagerCache, SharedCache, &["deno"];
    "Puppeteer browsers" => ".cache/puppeteer", PackageManagerCache, SharedCache, &[];
    "Playwright browsers" => ".cache/ms-playwright", PackageManagerCache, SharedCache, &[];
    "uv cache" => ".cache/uv", PackageManagerCache, SharedCache, &["uv", "uvx"];

    "Unity package cache" => ".cache/Unity/upm", UnityLibrary, SharedCache, &["Unity"];
    "Unity Asset Store packages" => ".local/share/unity3d/Asset Store-5.x", UnityLibrary, ToolState, &["Unity"];
    "Unity build cache" => ".cache/unity3d/bee", UnityLibrary, ToolState, &["Unity"];
    "Godot editor cache" => ".cache/godot", GodotCache, Artifact, &["Godot"];
    "Godot export templates" => ".local/share/godot/export_templates", GodotCache, SharedCache, &["Godot"];

    "Android system images" => "Android/Sdk/system-images", AndroidSdk, SharedCache, &["emulator", "qemu-system-x86_64", "studio"];
    "Android emulators" => ".android/avd", AndroidSdk, ReportOnly, &["emulator", "qemu-system-x86_64"];

    "Docker disk image" => ".docker/desktop/vms/0/data", Docker, ReportOnly, &["com.docker.backend", "docker"];

    "nvm download cache" => ".nvm/.cache", PackageManagerCache, SharedCache, &[];
    "Node versions (nvm)" => ".nvm/versions/node", PackageManagerCache, ToolState, &["node"];
    "pyenv build cache" => ".pyenv/cache", PackageManagerCache, SharedCache, &[];
    "Python versions (pyenv)" => ".pyenv/versions", PackageManagerCache, ToolState, &["python", "python3"];

    "Claude Code shell snapshots" => ".claude/shell-snapshots", AiToolCache, ToolState, &["claude"];
    "Claude Code IDE locks" => ".claude/ide", AiToolCache, ToolState, &["claude"];
    "Claude Code session environment" => ".claude/session-env", AiToolCache, ToolState, &["claude"];
    "Codex logs" => ".codex/log", AiToolCache, ToolState, &["codex"];
    "opencode cache" => ".cache/opencode", AiToolCache, ToolState, &["opencode"];
    "opencode logs" => ".local/share/opencode/log", AiToolCache, ToolState, &["opencode"];
    "opencode tool output" => ".local/share/opencode/tool-output", AiToolCache, ToolState, &["opencode"];

    "JetBrains caches" => ".cache/JetBrains", EditorCache, ToolState, &["idea", "pycharm", "webstorm", "goland", "clion", "phpstorm", "rubymine", "rustrover", "datagrip"];
    "VS Code cache" => ".config/Code/Cache", EditorCache, Artifact, &["Code"];
    "VS Code cached data" => ".config/Code/CachedData", EditorCache, Artifact, &["Code"];
    "VS Code GPU cache" => ".config/Code/GPUCache", EditorCache, Artifact, &["Code"];
};

#[cfg(windows)]
static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "npm cache" => "AppData/Local/npm-cache/_cacache", PackageManagerCache, SharedCache, &[];
    "pnpm store" => "AppData/Local/pnpm/store", PackageManagerCache, SharedCache, &[];
    "Yarn cache" => "AppData/Local/Yarn/Cache", PackageManagerCache, SharedCache, &[];
    "Yarn Berry cache" => ".yarn/berry/cache", PackageManagerCache, SharedCache, &[];
    "Bun cache" => ".bun/install/cache", PackageManagerCache, SharedCache, &["bun"];
    "Cargo registry" => ".cargo/registry", PackageManagerCache, SharedCache, &["cargo"];
    "Gradle cache" => ".gradle/caches", PackageManagerCache, SharedCache, &[];
    "Maven repository" => ".m2/repository", PackageManagerCache, ToolState, &[];
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache, &[];
    "Go module cache" => "go/pkg/mod", PackageManagerCache, ToolState, &["go"];
    "Go build cache" => "AppData/Local/go-build", PackageManagerCache, SharedCache, &["go"];
    "pip cache" => "AppData/Local/pip/Cache", PackageManagerCache, SharedCache, &[];
    "node-gyp headers" => "AppData/Local/node-gyp/Cache", PackageManagerCache, SharedCache, &[];
    "Deno cache" => "AppData/Local/deno", PackageManagerCache, SharedCache, &["deno"];
    "Puppeteer browsers" => ".cache/puppeteer", PackageManagerCache, SharedCache, &[];
    "Playwright browsers" => "AppData/Local/ms-playwright", PackageManagerCache, SharedCache, &[];
    "uv cache" => "AppData/Local/uv/cache", PackageManagerCache, SharedCache, &["uv", "uvx"];

    // Godot's own cache directory is the one path its documentation gives two
    // answers for, so it is not catalogued here: `%TEMP%\\Godot` in the data
    // paths guide, `%LOCALAPPDATA%\\Godot` in the class reference. Export
    // templates are stated once and agree with the other platforms.
    "Unity package cache" => "AppData/Local/Unity/cache/upm", UnityLibrary, SharedCache, &["Unity"];
    "Unity Asset Store packages" => "AppData/Roaming/Unity/Asset Store-5.x", UnityLibrary, ToolState, &["Unity"];
    "Unity build cache" => "AppData/Local/Unity/Caches/bee", UnityLibrary, ToolState, &["Unity"];
    "Godot export templates" => "AppData/Roaming/Godot/export_templates", GodotCache, SharedCache, &["Godot"];

    // The emulator directory is left out here: Android's own documentation
    // prints two different Windows layouts for it, and neither agrees with the
    // SDK root it documents elsewhere.
    "Android system images" => "AppData/Local/Android/Sdk/system-images", AndroidSdk, SharedCache, &["emulator", "qemu-system-x86_64", "studio"];

    "Docker disk image" => "AppData/Local/Docker/wsl/data", Docker, ReportOnly, &["com.docker.backend", "Docker Desktop", "docker"];

    "Ollama models" => ".ollama/models", AiToolCache, ToolState, &["ollama"];

    "Claude Code shell snapshots" => ".claude/shell-snapshots", AiToolCache, ToolState, &["claude"];
    "Claude Code IDE locks" => ".claude/ide", AiToolCache, ToolState, &["claude"];
    "Claude Code session environment" => ".claude/session-env", AiToolCache, ToolState, &["claude"];
    "Codex logs" => ".codex/log", AiToolCache, ToolState, &["codex"];
    "opencode cache" => ".cache/opencode", AiToolCache, ToolState, &["opencode"];

    "JetBrains caches" => "AppData/Local/JetBrains", EditorCache, ToolState, &["idea", "pycharm", "webstorm", "goland", "clion", "phpstorm", "rubymine", "rustrover", "datagrip"];
    "VS Code cache" => "AppData/Roaming/Code/Cache", EditorCache, Artifact, &["Code"];
    "VS Code cached data" => "AppData/Roaming/Code/CachedData", EditorCache, Artifact, &["Code"];
    "VS Code GPU cache" => "AppData/Roaming/Code/GPUCache", EditorCache, Artifact, &["Code"];
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

    /// A directory outside the home tree, standing in for a second volume.
    #[cfg(unix)]
    struct Elsewhere(PathBuf);

    #[cfg(unix)]
    impl Elsewhere {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("uncached-elsewhere-{name}"));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join("pnpm-cache")).expect("target fixture should be created");
            Self(path.join("pnpm-cache"))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(unix)]
    impl Drop for Elsewhere {
        fn drop(&mut self) {
            if let Some(parent) = self.0.parent() {
                let _ = fs::remove_dir_all(parent);
            }
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
            crate::filesystem::canonical(&expected).expect("fixture should resolve"),
            "discoveries are reported resolved, like traversal reports its own"
        );
        assert_eq!(found[0].label, location.label);
    }

    /// A relocated cache: moved to a second volume, reached through the link.
    ///
    /// The whole reason the entry is worth finding is that it is large enough
    /// to have been moved.
    #[cfg(unix)]
    #[test]
    fn a_catalogued_entry_moved_elsewhere_is_still_found() {
        let home = Home::new("relocated");
        let elsewhere = Elsewhere::new("relocated-target");
        let location = first_of_kind(LocationKind::SharedCache);
        let link = location.resolve(home.path());
        fs::create_dir_all(link.parent().expect("location should have a parent"))
            .expect("parent should be created");
        std::os::unix::fs::symlink(elsewhere.path(), &link).expect("link should be created");

        let found = probe_home(home.path());

        assert_eq!(
            found.len(),
            1,
            "a moved cache is still the catalogued entry"
        );
        assert_eq!(
            found[0].path,
            crate::filesystem::canonical(elsewhere.path()).expect("target should resolve"),
            "the discovery names where the directory actually is"
        );
        assert_eq!(
            found[0].root,
            crate::filesystem::canonical(elsewhere.path())
                .expect("target should resolve")
                .parent()
                .expect("target should have a parent")
                .to_path_buf(),
            "cleanup checks containment against the base it landed in, not home"
        );
    }

    /// Discovering it is worth nothing if cleanup then refuses it: the target
    /// is offered by one gate and rejected by the next, which is the failure
    /// the `root` field was introduced to stop.
    #[cfg(unix)]
    #[test]
    fn a_relocated_entry_is_vouched_for_by_its_provider() {
        let home = Home::new("relocated-vouch");
        let elsewhere = Elsewhere::new("relocated-vouch-target");
        let location = first_of_kind(LocationKind::SharedCache);
        let link = location.resolve(home.path());
        fs::create_dir_all(link.parent().expect("location should have a parent"))
            .expect("parent should be created");
        std::os::unix::fs::symlink(elsewhere.path(), &link).expect("link should be created");

        let found = probe_home(home.path());
        let discovery = found.first().expect("the relocated entry should be found");
        let providers = KnownLocationScanner::provider_set(home.path().to_path_buf());
        let scanner = providers
            .scanner_for(&discovery.category)
            .expect("the category should have a provider");

        let name = discovery
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the target should have a name");
        assert!(
            scanner.matches(name),
            "the directory it was moved into is named nothing like the entry"
        );
        assert!(
            scanner.confirm_context(&discovery.path),
            "the provider has to vouch for the path cleanup will be handed"
        );
    }

    /// The gate that actually matters: cleanup has to accept what discovery
    /// offered. A target the first gate reports and the second refuses is
    /// worse than one never reported at all — the user is shown space they
    /// cannot reclaim, and the refusal reads as a bug in the product.
    #[cfg(unix)]
    #[test]
    fn cleanup_accepts_a_relocated_entry_it_was_offered() {
        use crate::model::ScanTarget;

        let home = Home::new("relocated-cleanup");
        let elsewhere = Elsewhere::new("relocated-cleanup-target");
        let location = first_of_kind(LocationKind::SharedCache);
        let link = location.resolve(home.path());
        fs::create_dir_all(link.parent().expect("location should have a parent"))
            .expect("parent should be created");
        std::os::unix::fs::symlink(elsewhere.path(), &link).expect("link should be created");

        let found = probe_home(home.path());
        let discovery = found.first().expect("the relocated entry should be found");
        let providers = KnownLocationScanner::provider_set(home.path().to_path_buf());
        let target = ScanTarget::new(discovery.path.clone(), discovery.category.clone(), 0)
            .with_identity(crate::filesystem::file_identity(&discovery.path));

        let validated = crate::cleaner::validate_clean_target(&discovery.root, &providers, &target);

        assert!(
            validated.is_ok(),
            "cleanup refused a relocated cache discovery offered: {:?}",
            validated.err()
        );
    }

    /// The Windows half of the same fact.
    ///
    /// A relocated cache here is usually a junction, whose reparse tag is
    /// `MOUNT_POINT` rather than `SYMLINK` — which is why [`is_redirect`] reads
    /// the attribute instead of trusting `is_symlink`. `junction_point` is
    /// still unstable, so the test uses a directory symlink: it exercises the
    /// same attribute bit through the same code path.
    ///
    /// Creating one needs Developer Mode or an elevated process. Where neither
    /// holds, the test says so and stops rather than failing — the same shape
    /// the Trash backend tests use when the platform will not cooperate.
    #[cfg(windows)]
    #[test]
    fn a_catalogued_entry_moved_to_another_volume_is_still_found() {
        let home = Home::new("relocated-windows");
        let elsewhere = std::env::temp_dir().join("uncached-elsewhere-windows/pnpm-cache");
        let _ = fs::remove_dir_all(elsewhere.parent().expect("target should have a parent"));
        fs::create_dir_all(&elsewhere).expect("target fixture should be created");

        let location = first_of_kind(LocationKind::SharedCache);
        let link = location.resolve(home.path());
        fs::create_dir_all(link.parent().expect("location should have a parent"))
            .expect("parent should be created");

        if std::os::windows::fs::symlink_dir(&elsewhere, &link).is_err() {
            eprintln!("skipping: creating a directory link needs Developer Mode or elevation");
            let _ = fs::remove_dir_all(elsewhere.parent().expect("target should have a parent"));
            return;
        }

        let found = probe_home(home.path());

        assert_eq!(
            found.len(),
            1,
            "a moved cache is still the catalogued entry"
        );
        assert_eq!(
            found[0].path,
            crate::filesystem::canonical(&elsewhere).expect("target should resolve"),
            "the discovery names where the directory actually is"
        );

        let providers = KnownLocationScanner::provider_set(home.path().to_path_buf());
        let scanner = providers
            .scanner_for(&found[0].category)
            .expect("the category should have a provider");
        let name = found[0]
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the target should have a name");
        assert!(crate::scanner::Scanner::matches(scanner, name));
        assert!(crate::scanner::Scanner::confirm_context(
            scanner,
            &found[0].path
        ));

        let _ = fs::remove_dir_all(elsewhere.parent().expect("target should have a parent"));
    }

    /// A link is only the entry if it leads to one.
    #[cfg(unix)]
    #[test]
    fn a_link_to_a_file_or_to_nothing_is_not_reported() {
        let home = Home::new("relocated-broken");
        let location = first_of_kind(LocationKind::SharedCache);
        let link = location.resolve(home.path());
        fs::create_dir_all(link.parent().expect("location should have a parent"))
            .expect("parent should be created");

        let file = home.path().join("a-file");
        fs::write(&file, b"not a cache").expect("file fixture should be written");
        std::os::unix::fs::symlink(&file, &link).expect("link should be created");
        assert!(
            probe_home(home.path()).is_empty(),
            "a link to a file is not the entry"
        );

        fs::remove_file(&link).expect("link should be removed");
        std::os::unix::fs::symlink(home.path().join("gone"), &link)
            .expect("dangling link should be created");
        assert!(
            probe_home(home.path()).is_empty(),
            "a dangling link is not the entry"
        );
    }

    /// Moving a cache without leaving home keeps the narrower base.
    #[cfg(unix)]
    #[test]
    fn a_redirect_that_stays_inside_home_keeps_home_as_its_root() {
        let home = Home::new("relocated-inside");
        let location = first_of_kind(LocationKind::SharedCache);
        let target = home.create("moved-cache");
        let link = location.resolve(home.path());
        fs::create_dir_all(link.parent().expect("location should have a parent"))
            .expect("parent should be created");
        std::os::unix::fs::symlink(&target, &link).expect("link should be created");

        let found = probe_home(home.path());

        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].root,
            crate::filesystem::canonical(home.path()).expect("home should resolve"),
            "a redirection inside home does not need to widen the base"
        );
    }

    fn system_location(path: &Path) -> super::SystemLocation {
        super::SystemLocation {
            label: "Simulator runtimes",
            path: Box::leak(path.to_string_lossy().into_owned().into_boxed_str()),
            category: Category::XcodeDerivedData,
            owner: &["Xcode"],
            reclaim_with: "xcrun simctl runtime delete --notUsedSinceDays 30",
        }
    }

    /// Found outside home, and refused by cleanup at the provider gate rather
    /// than by anything the interface decides.
    #[test]
    fn a_system_location_is_reported_and_cleanup_refuses_it() {
        use crate::model::ScanTarget;

        let home = Home::new("system-home");
        let machine = Home::new("system-machine");
        let runtimes = machine.create("AssetsV2/com_apple_MobileAsset_iOSSimulatorRuntime");

        let found = super::probe_absolute(&[system_location(&runtimes)]);

        assert_eq!(found.len(), 1);
        let discovery = &found[0];
        assert_eq!(discovery.kind, LocationKind::ReportOnly);
        assert_eq!(
            discovery.reclaim_with,
            Some("xcrun simctl runtime delete --notUsedSinceDays 30")
        );
        assert!(crate::filesystem::is_within(
            &discovery.root,
            &discovery.path
        ));

        let providers = KnownLocationScanner::provider_set(home.path().to_path_buf());
        let name = discovery
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the target should have a name");
        assert!(providers.classify(name, &discovery.path).is_none());

        let target = ScanTarget::new(discovery.path.clone(), discovery.category.clone(), 0)
            .with_identity(crate::filesystem::file_identity(&discovery.path));
        assert!(
            crate::cleaner::validate_clean_target(&discovery.root, &providers, &target).is_err(),
            "a machine-wide location must never be removable"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_catalogued_report_only_entry_carries_its_owners_command() {
        let home = Home::new("reclaim-with");
        home.create("Library/Developer/CoreSimulator/Devices");
        home.create("Library/Developer/Xcode/DerivedData");

        let found = probe_home(home.path());
        let command = |label: &str| {
            found
                .iter()
                .find(|discovery| discovery.label == label)
                .expect("fixture should be found")
                .reclaim_with
        };

        assert_eq!(
            command("Simulator devices"),
            Some("xcrun simctl delete unavailable")
        );
        assert_eq!(command("Xcode DerivedData"), None);

        home.create(".android/avd");
        home.create("Library/Containers/com.docker.docker/Data/vms/0/data");
        let found = probe_home(home.path());
        let command = |label: &str| {
            found
                .iter()
                .find(|discovery| discovery.label == label)
                .expect("fixture should be found")
                .reclaim_with
        };
        assert_eq!(
            command("Android emulators"),
            Some("avdmanager delete avd -n <name>")
        );
        assert_eq!(command("Docker disk image"), Some("docker system prune"));
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_or_redirected_system_location_is_skipped() {
        let machine = Home::new("system-skipped");
        let real = machine.create("real");
        let link = machine.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("link should be created");

        let found = super::probe_absolute(&[
            system_location(&machine.path().join("absent")),
            system_location(&link),
        ]);

        assert!(found.is_empty());
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

    /// The prefix alone is not enough: `claude-config` is somebody's settings.
    #[cfg(unix)]
    #[test]
    fn only_a_user_id_suffix_is_a_session_scratch_directory() {
        let root = std::env::temp_dir().join(format!("uncached-temp-probe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        // `probe_temp` reads the real temp roots, so this exercises the naming
        // rule directly rather than through the filesystem.
        for (name, expected) in [
            ("claude-501", true),
            ("claude-0", true),
            ("claude-", false),
            ("claude-config", false),
            ("claude-501x", false),
            ("claudex-501", false),
        ] {
            assert_eq!(
                super::is_session_scratch(name, "claude-"),
                expected,
                "{name}"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// A scratch directory is nothing unless cleanup will act on it, and
    /// cleanup only acts on paths a provider vouches for.
    #[test]
    fn a_probed_scratch_directory_is_vouched_for() {
        let Some(home) = super::home_directory() else {
            return;
        };
        let scratch = super::probe_temp(&home);
        if scratch.is_empty() {
            return;
        }
        let providers = KnownLocationScanner::provider_set(home);
        for discovery in scratch {
            let name = discovery
                .path
                .file_name()
                .and_then(|segment| segment.to_str())
                .expect("a probed directory has a name");
            assert!(
                providers.classify(name, &discovery.path).is_some(),
                "{} was probed and then refused by every provider",
                discovery.path.display()
            );
        }
    }

    /// The bug this field exists for.
    ///
    /// Cleanup refuses a target that is not inside the root it was registered
    /// against. Every catalogued location used to be registered against the
    /// home directory, which was true until session scratch — that lives under
    /// `/tmp`, so it was offered on screen and then refused on removal, with
    /// nothing on either side explaining why.
    #[cfg(unix)]
    #[test]
    fn every_discovery_is_inside_the_root_it_reports() {
        // Imported here rather than at the top of the module: both are only
        // used by this test, which does not exist off unix.
        use super::{home_directory, probe_temp};

        let Some(home) = home_directory() else {
            return;
        };
        let mut found = probe_home(&home);
        found.extend(probe_temp(&home));
        found.extend(super::probe_system());
        for discovery in &found {
            assert!(
                discovery.path.starts_with(&discovery.root),
                "{} reports a root that does not contain it: {}",
                discovery.path.display(),
                discovery.root.display()
            );
        }
    }

    #[test]
    fn a_shared_cache_never_reaches_high() {
        let discovery = Discovery {
            path: PathBuf::from("/home/user/.npm/_cacache"),
            label: "npm cache",
            category: Category::PackageManagerCache,
            kind: LocationKind::SharedCache,
            owner: &[],
            root: PathBuf::from("/home/user"),
            reclaim_with: None,
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
            owner: &["claude"],
            root: PathBuf::from("/home/user"),
            reclaim_with: None,
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
            owner: &["Xcode"],
            root: PathBuf::from("/home/user"),
            reclaim_with: None,
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
            // Removing the version in use breaks the environment until it is
            // installed again, and neither tool names which that is on disk.
            ".nvm/versions/node",
            ".pyenv/versions",
            // Re-pullable, but tens of gigabytes over a network.
            ".ollama/models",
            // A package delisted from the Asset Store cannot be downloaded
            // again, and this cache holds the last copy of it.
            "Library/Unity/Asset Store-5.x",
            ".local/share/unity3d/Asset Store-5.x",
            "AppData/Roaming/Unity/Asset Store-5.x",
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
