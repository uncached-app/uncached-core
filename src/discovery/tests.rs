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
