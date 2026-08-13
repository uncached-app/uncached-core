//! Confidence assessment for cleanup candidates.
//!
//! The scanner answers "is this a build artifact?". This module answers a
//! different question: "how sure are we?". Keeping the two apart is what lets
//! the product search aggressively and delete conservatively.
//!
//! Assessment is a pure function of collected signals. It performs no I/O, so
//! the code that decides whether something may be deleted can be tested
//! exhaustively without a filesystem, and behaves identically on every caller.

use crate::filesystem::has_marker_file;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Manifests that mark the parent directory as a project root.
const PROJECT_MANIFESTS: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
];

/// Lockfiles that make the contents reproducible.
const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
    "bun.lock",
    "Cargo.lock",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
];

/// Directories owned by a runtime version manager. What lives under them was
/// installed by the user, not produced by a build.
const VERSION_MANAGER_DIRECTORIES: &[&str] = &[
    ".nvm", ".volta", ".fnm", ".asdf", ".rbenv", ".pyenv", ".nodenv", ".rustup", ".sdkman",
];

/// Roots owned by the operating system rather than by a user's projects.
///
/// `/var` is deliberately absent: macOS keeps per-user temporary directories
/// under `/private/var`, so treating it as a system path would mislabel
/// ordinary scratch space. Deletion under critical directories is refused by
/// `filesystem::is_safe_to_delete` regardless of what confidence reports.
#[cfg(not(windows))]
const SYSTEM_ROOTS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/etc",
    "/opt",
    "/System",
    "/Library",
    "/Applications",
];

const RECENT_WRITE_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

/// How far up the tree to look for `.gitignore` files before giving up. Deep
/// enough for a monorepo, bounded so a pathological path cannot walk forever.
const MAX_IGNORE_DEPTH: usize = 32;

/// Answers whether the project declares this directory disposable, reading the
/// `.gitignore` files between the candidate and its repository root.
///
/// This deliberately understands only unambiguous patterns: a bare name, a
/// trailing slash, a rooted path, a `**/` prefix. Anything else — globs,
/// character classes, a negation touching the same name — makes the file's
/// intent unclear to a matcher this simple, and the answer becomes `None`.
///
/// That asymmetry is the point. `Some(true)` means the project said so and the
/// signal can be trusted. `None` means "not established", never "not ignored",
/// so a pattern we cannot parse can only cost a candidate corroboration it
/// might have earned, never mark it as undeclared. Full `.gitignore` semantics
/// would need Git itself or a matching library; neither is worth a subprocess
/// per candidate or a dependency for a corroborating signal.
pub fn gitignore_signal(path: &Path) -> Option<bool> {
    let name = path.file_name()?.to_str()?;
    let mut directory = path.parent()?;

    for _ in 0..MAX_IGNORE_DEPTH {
        if let Some(declared) = declares_disposable(&directory.join(".gitignore"), name) {
            return Some(declared).filter(|declared| *declared);
        }
        if directory.join(".git").exists() {
            return None;
        }
        directory = directory.parent()?;
    }
    None
}

/// `Some(true)` when the file clearly ignores `name`, `Some(false)` when it
/// clearly un-ignores it, `None` when this file says nothing usable about it.
fn declares_disposable(ignore_file: &Path, name: &str) -> Option<bool> {
    let contents = std::fs::read_to_string(ignore_file).ok()?;
    let mut declared = None;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (negated, pattern) = match line.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let Some(pattern) = unambiguous_pattern(pattern) else {
            continue;
        };
        if pattern == name {
            // Later lines win in Git, so keep reading instead of returning.
            declared = Some(!negated);
        }
    }

    declared
}

/// Strips the decorations this matcher understands and rejects everything else.
fn unambiguous_pattern(pattern: &str) -> Option<&str> {
    let pattern = pattern.strip_prefix("**/").unwrap_or(pattern);
    let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
    let pattern = pattern.strip_suffix('/').unwrap_or(pattern);

    let ambiguous = pattern.is_empty()
        || pattern.contains(['*', '?', '[', ']', '/', '\\'])
        || pattern.starts_with('#');

    (!ambiguous).then_some(pattern)
}

/// Gathers the signals `assess` weighs.
///
/// Reads the filesystem around the candidate and spawns no process. Whether the
/// directory is tracked by Git is left unchecked: answering it means running
/// Git, and an unchecked signal is honest while a guessed one is not. A caller
/// that can afford the lookup fills that field before assessing.
pub fn collect_signals(path: &Path, known_artifact: bool) -> ConfidenceInput {
    let parent = path.parent();

    ConfidenceInput {
        known_artifact,
        project_manifest: parent.is_some_and(|parent| has_any_marker(parent, PROJECT_MANIFESTS)),
        lockfile: parent.is_some_and(|parent| has_any_marker(parent, LOCKFILES)),
        git_ignored: gitignore_signal(path),
        git_tracked: None,
        version_manager_path: is_version_manager_path(path),
        system_path: is_system_path(path),
        recently_modified: was_modified_recently(path),
    }
}

fn has_any_marker(parent: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| has_marker_file(parent, name))
}

fn is_version_manager_path(path: &Path) -> bool {
    let components = lowercase_components(path);

    if components
        .iter()
        .any(|component| VERSION_MANAGER_DIRECTORIES.contains(&component.as_str()))
    {
        return true;
    }

    // nvm-windows and older installs drop the leading dot, so the directory
    // name alone is too common to trust. A version directory next to it is not.
    components.windows(2).any(|window| {
        (window[0] == "nvm" && is_version_directory(&window[1]))
            || (window[0] == "pnpm" && window[1] == "global")
            || (window[0] == ".bun" && window[1] == "install")
    })
}

fn is_version_directory(name: &str) -> bool {
    let digits = name.strip_prefix('v').unwrap_or(name);
    !digits.is_empty()
        && digits
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
}

#[cfg(not(windows))]
fn is_system_path(path: &Path) -> bool {
    SYSTEM_ROOTS
        .iter()
        .any(|root| path.starts_with(root) && path != Path::new(root))
}

#[cfg(windows)]
fn is_system_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().to_lowercase();
    [
        std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string()),
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string()),
        std::env::var("ProgramFiles(x86)")
            .unwrap_or_else(|_| r"C:\Program Files (x86)".to_string()),
    ]
    .iter()
    .any(|root| {
        let root = root.trim_end_matches(['\\', '/']).to_lowercase();
        normalized.starts_with(&(root + "\\"))
    })
}

fn lowercase_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

fn was_modified_recently(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|elapsed| elapsed < RECENT_WRITE_WINDOW)
}

/// What the user sees. The score behind it stays internal so thresholds can be
/// tuned without turning into a number the UI has to explain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfidenceLevel {
    /// Not identifiable as a project artifact. Show it, explain it, delete
    /// nothing without the user looking first.
    Review,
    /// Plausible, but something about it warrants a second look.
    Medium,
    /// Identified as regenerable, inside a project that says so.
    High,
}

/// A single reason behind an assessment. These are the sentences the UI shows,
/// so the product can always explain itself instead of asserting a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfidenceSignal {
    /// A provider recognized the directory as a known artifact type.
    KnownArtifact,
    /// A project manifest sits next to it.
    ProjectManifest,
    /// A lockfile sits next to it, so the contents can be reproduced.
    Lockfile,
    /// The project's own `.gitignore` declares it disposable.
    DeclaredDisposable,
    /// The directory is tracked by Git, so it is not a build output.
    VersionControlled,
    /// A version manager owns this path: it holds globally installed packages.
    VersionManagerPath,
    /// The path belongs to the operating system rather than to a project.
    SystemPath,
    /// Written to very recently, so something may still be using it.
    RecentlyModified,
    /// No project could be associated with it.
    NoProjectContext,
}

impl ConfidenceSignal {
    /// The explanation shown next to the badge.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::KnownArtifact => "Recognized build artifact",
            Self::ProjectManifest => "Project manifest found alongside it",
            Self::Lockfile => "Lockfile found, contents are reproducible",
            Self::DeclaredDisposable => "Declared disposable by the project's .gitignore",
            Self::VersionControlled => "Tracked by Git, so it is not a build output",
            Self::VersionManagerPath => {
                "Version manager directory, holds globally installed packages"
            }
            Self::SystemPath => "Belongs to the operating system, not to a project",
            Self::RecentlyModified => "Written to in the last day",
            Self::NoProjectContext => "No project found around it",
        }
    }

    /// Whether the signal argues for deleting or against it. The UI uses this
    /// to mark each reason rather than inferring tone from wording.
    pub fn is_supporting(&self) -> bool {
        matches!(
            self,
            Self::KnownArtifact | Self::ProjectManifest | Self::Lockfile | Self::DeclaredDisposable
        )
    }
}

/// Signals gathered by the caller. Every field is a fact about the candidate,
/// never a conclusion: the weighing happens here so it happens once.
///
/// `Option` fields distinguish "checked and false" from "not checked". An
/// unchecked signal never counts against a candidate, but it also never earns
/// it the benefit of the doubt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConfidenceInput {
    /// A provider matched the directory name and confirmed its context.
    pub known_artifact: bool,
    /// A manifest such as `package.json` or `Cargo.toml` sits in the parent.
    pub project_manifest: bool,
    /// A lockfile sits in the parent.
    pub lockfile: bool,
    /// Covered by the project's `.gitignore`. `None` when not checked.
    pub git_ignored: Option<bool>,
    /// Tracked by Git. `None` when not checked.
    pub git_tracked: Option<bool>,
    /// Sits under a version manager such as nvm, rbenv or asdf. A
    /// `node_modules` there holds the user's globally installed packages, not a
    /// project's build output.
    pub version_manager_path: bool,
    /// Sits under an operating system directory.
    pub system_path: bool,
    /// Modified within the last day.
    pub recently_modified: bool,
}

/// The result of weighing the signals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Confidence {
    pub level: ConfidenceLevel,
    pub signals: Vec<ConfidenceSignal>,
}

impl Confidence {
    pub fn is_high(&self) -> bool {
        self.level == ConfidenceLevel::High
    }
}

const SCORE_KNOWN_ARTIFACT: i32 = 40;
const SCORE_PROJECT_MANIFEST: i32 = 30;
const SCORE_DECLARED_DISPOSABLE: i32 = 25;
const SCORE_LOCKFILE: i32 = 15;
const PENALTY_RECENTLY_MODIFIED: i32 = 10;

/// A recognized artifact sitting next to a project manifest reaches High on its
/// own. That pair is what the scanner already requires before offering a
/// directory for deletion, so demanding more would mark ordinary targets for
/// review and teach the user to ignore the badge. Lockfiles and `.gitignore`
/// corroborate; they are not entry requirements.
const THRESHOLD_HIGH: i32 = 70;
const THRESHOLD_MEDIUM: i32 = 45;

/// Weighs the signals into a level and the reasons behind it.
///
/// Three signals override the score entirely, because no amount of supporting
/// evidence makes them safe:
///
/// - a directory tracked by Git is not a build output, whatever it is named;
/// - a path owned by the operating system is never a project artifact;
/// - a version manager directory holds what the user installed globally.
///
/// The last one is the easiest to get wrong. A `node_modules` under
/// `~/.nvm/versions/node/*/lib` is not a project's dependencies: it is the
/// packages installed with `npm i -g` for that runtime. Removing it uninstalls
/// the user's tooling, and no build step brings it back.
pub fn assess(input: ConfidenceInput) -> Confidence {
    let mut signals = Vec::new();
    let mut score = 0;

    if input.known_artifact {
        signals.push(ConfidenceSignal::KnownArtifact);
        score += SCORE_KNOWN_ARTIFACT;
    }
    if input.project_manifest {
        signals.push(ConfidenceSignal::ProjectManifest);
        score += SCORE_PROJECT_MANIFEST;
    } else {
        signals.push(ConfidenceSignal::NoProjectContext);
    }
    if input.lockfile {
        signals.push(ConfidenceSignal::Lockfile);
        score += SCORE_LOCKFILE;
    }
    if input.git_ignored == Some(true) {
        signals.push(ConfidenceSignal::DeclaredDisposable);
        score += SCORE_DECLARED_DISPOSABLE;
    }
    if input.recently_modified {
        signals.push(ConfidenceSignal::RecentlyModified);
        score -= PENALTY_RECENTLY_MODIFIED;
    }
    if input.version_manager_path {
        signals.push(ConfidenceSignal::VersionManagerPath);
    }
    if input.system_path {
        signals.push(ConfidenceSignal::SystemPath);
    }
    if input.git_tracked == Some(true) {
        signals.push(ConfidenceSignal::VersionControlled);
    }

    let vetoed = input.git_tracked == Some(true) || input.system_path || input.version_manager_path;
    let level = if vetoed {
        ConfidenceLevel::Review
    } else {
        level_for_score(score)
    };

    Confidence { level, signals }
}

fn level_for_score(score: i32) -> ConfidenceLevel {
    if score >= THRESHOLD_HIGH {
        ConfidenceLevel::High
    } else if score >= THRESHOLD_MEDIUM {
        ConfidenceLevel::Medium
    } else {
        ConfidenceLevel::Review
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_artifact() -> ConfidenceInput {
        ConfidenceInput {
            known_artifact: true,
            project_manifest: true,
            lockfile: true,
            git_ignored: Some(true),
            git_tracked: Some(false),
            ..ConfidenceInput::default()
        }
    }

    #[test]
    fn an_artifact_inside_a_project_that_ignores_it_is_high() {
        let confidence = assess(project_artifact());

        assert_eq!(confidence.level, ConfidenceLevel::High);
        assert!(confidence.is_high());
        assert!(confidence
            .signals
            .contains(&ConfidenceSignal::DeclaredDisposable));
        assert!(!confidence
            .signals
            .contains(&ConfidenceSignal::NoProjectContext));
    }

    #[test]
    fn an_unrecognized_directory_with_no_project_is_review() {
        let confidence = assess(ConfidenceInput::default());

        assert_eq!(confidence.level, ConfidenceLevel::Review);
        assert_eq!(confidence.signals, vec![ConfidenceSignal::NoProjectContext]);
    }

    #[test]
    fn version_control_vetoes_every_supporting_signal() {
        let confidence = assess(ConfidenceInput {
            git_tracked: Some(true),
            ..project_artifact()
        });

        assert_eq!(confidence.level, ConfidenceLevel::Review);
        assert!(confidence
            .signals
            .contains(&ConfidenceSignal::VersionControlled));
    }

    #[test]
    fn a_system_path_vetoes_every_supporting_signal() {
        let confidence = assess(ConfidenceInput {
            system_path: true,
            ..project_artifact()
        });

        assert_eq!(confidence.level, ConfidenceLevel::Review);
        assert!(confidence.signals.contains(&ConfidenceSignal::SystemPath));
    }

    // A `node_modules` under a version manager holds packages installed with
    // `npm i -g`, not a project's dependencies. Nothing regenerates it, so it
    // is vetoed however strong the surrounding evidence looks.
    #[test]
    fn a_version_manager_path_vetoes_every_supporting_signal() {
        let confidence = assess(ConfidenceInput {
            version_manager_path: true,
            ..project_artifact()
        });

        assert_eq!(confidence.level, ConfidenceLevel::Review);
        assert!(confidence
            .signals
            .contains(&ConfidenceSignal::VersionManagerPath));
    }

    #[test]
    fn an_unchecked_gitignore_does_not_count_against_the_candidate() {
        let unchecked = assess(ConfidenceInput {
            git_ignored: None,
            git_tracked: None,
            ..project_artifact()
        });
        let checked_false = assess(ConfidenceInput {
            git_ignored: Some(false),
            git_tracked: Some(false),
            ..project_artifact()
        });

        assert_eq!(unchecked.level, checked_false.level);
        assert!(!unchecked
            .signals
            .contains(&ConfidenceSignal::DeclaredDisposable));
    }

    #[test]
    fn a_known_artifact_without_a_manifest_is_not_high() {
        let confidence = assess(ConfidenceInput {
            known_artifact: true,
            ..ConfidenceInput::default()
        });

        assert_eq!(confidence.level, ConfidenceLevel::Review);
        assert!(confidence
            .signals
            .contains(&ConfidenceSignal::NoProjectContext));
    }

    #[test]
    fn recent_writes_pull_a_borderline_candidate_down_a_level() {
        let minimal = ConfidenceInput {
            known_artifact: true,
            project_manifest: true,
            ..ConfidenceInput::default()
        };
        let settled = assess(minimal);
        let recent = assess(ConfidenceInput {
            recently_modified: true,
            ..minimal
        });

        assert_eq!(settled.level, ConfidenceLevel::High);
        assert_eq!(recent.level, ConfidenceLevel::Medium);
        assert!(recent.signals.contains(&ConfidenceSignal::RecentlyModified));
    }

    // The pair the scanner already requires before offering a directory for
    // deletion. A lockfile is not committed in every ecosystem, so demanding
    // more would mark ordinary Gradle and Python targets for review and teach
    // the user to ignore the badge.
    #[test]
    fn a_recognized_artifact_next_to_a_manifest_is_high_on_its_own() {
        let confidence = assess(ConfidenceInput {
            known_artifact: true,
            project_manifest: true,
            ..ConfidenceInput::default()
        });

        assert_eq!(confidence.level, ConfidenceLevel::High);
    }

    // Corroborating evidence absorbs the recency penalty, otherwise every
    // artifact in a project worked on today would ask for review.
    #[test]
    fn recent_writes_do_not_downgrade_a_well_evidenced_candidate() {
        let confidence = assess(ConfidenceInput {
            recently_modified: true,
            ..project_artifact()
        });

        assert_eq!(confidence.level, ConfidenceLevel::High);
    }

    #[test]
    fn every_signal_explains_itself_and_declares_its_side() {
        let signals = [
            ConfidenceSignal::KnownArtifact,
            ConfidenceSignal::ProjectManifest,
            ConfidenceSignal::Lockfile,
            ConfidenceSignal::DeclaredDisposable,
            ConfidenceSignal::VersionControlled,
            ConfidenceSignal::VersionManagerPath,
            ConfidenceSignal::SystemPath,
            ConfidenceSignal::RecentlyModified,
            ConfidenceSignal::NoProjectContext,
        ];

        for signal in signals {
            assert!(!signal.reason().is_empty());
        }
        assert!(ConfidenceSignal::KnownArtifact.is_supporting());
        assert!(!ConfidenceSignal::VersionControlled.is_supporting());
        assert!(!ConfidenceSignal::NoProjectContext.is_supporting());
    }

    #[test]
    fn levels_serialize_as_the_lowercase_wire_form() {
        let confidence = assess(project_artifact());
        let value = serde_json::to_value(&confidence).unwrap();

        assert_eq!(value["level"], "high");
        assert_eq!(value["signals"][0], "knownArtifact");
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "uncached-confidence-{}-{}-{:?}",
            name,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn collects_manifest_and_lockfile_from_the_parent_directory() {
        let root = fixture("manifest");
        let project = root.join("app");
        let target = project.join("node_modules");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(project.join("package.json"), b"{}").unwrap();
        std::fs::write(project.join("pnpm-lock.yaml"), b"").unwrap();

        let input = collect_signals(&target, true);

        assert!(input.project_manifest);
        assert!(input.lockfile);
        assert!(!input.version_manager_path);
        // Git costs a subprocess, so collection leaves it unanswered.
        assert_eq!(input.git_ignored, None);
        assert_eq!(input.git_tracked, None);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_directory_without_a_project_around_it_collects_nothing_supporting() {
        let root = fixture("bare");
        let target = root.join("some-cache");
        std::fs::create_dir_all(&target).unwrap();

        let input = collect_signals(&target, false);

        assert!(!input.project_manifest);
        assert!(!input.lockfile);
        assert_eq!(assess(input).level, ConfidenceLevel::Review);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_freshly_written_directory_reads_as_recently_modified() {
        let root = fixture("recent");
        let target = root.join("node_modules");
        std::fs::create_dir_all(&target).unwrap();

        assert!(collect_signals(&target, true).recently_modified);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn global_install_directories_are_recognized_without_touching_disk() {
        let version_manager_paths = [
            "/home/dev/.nvm/versions/node/v20.11.0/lib/node_modules",
            "/home/dev/.volta/tools/image/node/20.11.0/lib/node_modules",
            "/home/dev/.asdf/installs/nodejs/20.11.0/lib/node_modules",
            "/home/dev/.fnm/node-versions/v20.11.0/installation/lib/node_modules",
            "/home/dev/.local/share/pnpm/global/5/node_modules",
            "/home/dev/.bun/install/global/node_modules",
        ];

        for path in version_manager_paths {
            let input = collect_signals(Path::new(path), true);
            assert!(input.version_manager_path, "not detected: {path}");
            assert_eq!(assess(input).level, ConfidenceLevel::Review, "{path}");
        }
    }

    // Only Windows splits this into components; on Unix the whole string is a
    // single component and the path is meaningless.
    #[cfg(windows)]
    #[test]
    fn nvm_for_windows_is_recognized_by_its_version_directory() {
        let input = collect_signals(
            Path::new(r"C:\Users\dev\AppData\Roaming\nvm\v20.11.0\node_modules"),
            true,
        );

        assert!(input.version_manager_path);
        assert_eq!(assess(input).level, ConfidenceLevel::Review);
    }

    #[test]
    fn a_project_named_after_a_tool_is_not_a_version_manager_path() {
        for path in [
            "/home/dev/Projects/nvm/node_modules",
            "/home/dev/Projects/pnpm-utils/node_modules",
            "/home/dev/Projects/bun-app/node_modules",
        ] {
            assert!(
                !collect_signals(Path::new(path), true).version_manager_path,
                "false positive: {path}"
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn system_roots_are_flagged_but_ordinary_home_paths_are_not() {
        assert!(collect_signals(Path::new("/usr/lib/node_modules"), true).system_path);
        assert!(collect_signals(Path::new("/Library/Caches/node_modules"), true).system_path);
        assert!(!collect_signals(Path::new("/home/dev/app/node_modules"), true).system_path);
        // macOS keeps per-user temporary directories under /private/var.
        assert!(
            !collect_signals(Path::new("/private/var/folders/x/app/node_modules"), true)
                .system_path
        );
    }

    fn write_ignore(directory: &std::path::Path, contents: &str) {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join(".gitignore"), contents).unwrap();
    }

    #[test]
    fn a_project_that_ignores_the_directory_declares_it_disposable() {
        let root = fixture("ignored");
        let project = root.join("app");
        let target = project.join("node_modules");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(project.join(".git")).unwrap();
        write_ignore(&project, "# deps\nnode_modules\ndist\n");

        assert_eq!(gitignore_signal(&target), Some(true));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_decorations_git_users_actually_write_are_understood() {
        let root = fixture("decorations");
        for (index, pattern) in ["node_modules/", "/node_modules", "**/node_modules"]
            .iter()
            .enumerate()
        {
            let project = root.join(format!("app{index}"));
            let target = project.join("node_modules");
            std::fs::create_dir_all(&target).unwrap();
            std::fs::create_dir_all(project.join(".git")).unwrap();
            write_ignore(&project, pattern);

            assert_eq!(gitignore_signal(&target), Some(true), "{pattern}");
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_ignore_file_higher_up_still_counts() {
        let root = fixture("monorepo");
        let target = root.join("packages").join("web").join("node_modules");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        write_ignore(&root, "node_modules\n");

        assert_eq!(gitignore_signal(&target), Some(true));

        let _ = std::fs::remove_dir_all(root);
    }

    // The matcher must never claim a directory is undeclared. Not finding a
    // pattern it understands means "not established", so a candidate can only
    // lose corroboration it might have earned, never be marked as undeclared.
    #[test]
    fn anything_this_matcher_cannot_read_is_unanswered_rather_than_denied() {
        let root = fixture("unanswered");
        let cases = [
            ("no-ignore-file", None::<&str>),
            ("unrelated", Some("dist\nbuild\n")),
            ("glob", Some("node_modules*\n")),
            ("nested-path", Some("packages/*/node_modules\n")),
            ("negated", Some("node_modules\n!node_modules\n")),
            ("commented", Some("# node_modules\n")),
        ];

        for (name, contents) in cases {
            let project = root.join(name);
            let target = project.join("node_modules");
            std::fs::create_dir_all(&target).unwrap();
            std::fs::create_dir_all(project.join(".git")).unwrap();
            if let Some(contents) = contents {
                write_ignore(&project, contents);
            }

            assert_eq!(gitignore_signal(&target), None, "{name}");
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_later_line_wins_over_an_earlier_negation() {
        let root = fixture("relisted");
        let project = root.join("app");
        let target = project.join("node_modules");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(project.join(".git")).unwrap();
        write_ignore(&project, "!node_modules\nnode_modules\n");

        assert_eq!(gitignore_signal(&target), Some(true));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_search_stops_at_the_repository_root() {
        let root = fixture("bounded");
        let project = root.join("app");
        let target = project.join("node_modules");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(project.join(".git")).unwrap();
        // Outside the repository, so it must not be consulted.
        write_ignore(&root, "node_modules\n");

        assert_eq!(gitignore_signal(&target), None);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_declared_directory_collects_the_disposable_signal() {
        let root = fixture("collected");
        let project = root.join("app");
        let target = project.join("node_modules");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::write(project.join("package.json"), b"{}").unwrap();
        write_ignore(&project, "node_modules\n");

        let input = collect_signals(&target, true);
        assert_eq!(input.git_ignored, Some(true));
        assert!(assess(input)
            .signals
            .contains(&ConfidenceSignal::DeclaredDisposable));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn assessment_is_deterministic() {
        let input = project_artifact();
        assert_eq!(assess(input), assess(input));
    }
}
