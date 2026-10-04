use super::catalogue::known_locations;
use crate::confidence::ConfidenceInput;
use crate::model::{Category, LocationKind};
use std::path::{Path, PathBuf};

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
pub(super) fn is_session_scratch(name: &str, prefix: &str) -> bool {
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
pub(super) struct SystemLocation {
    pub(super) label: &'static str,
    pub(super) path: &'static str,
    pub(super) category: Category,
    pub(super) owner: &'static [&'static str],
    /// See [`Discovery::reclaim_with`].
    pub(super) reclaim_with: &'static str,
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
pub(super) fn probe_absolute(locations: &[SystemLocation]) -> Vec<Discovery> {
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
pub(super) fn is_redirect(metadata: &std::fs::Metadata) -> bool {
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
