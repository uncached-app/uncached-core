//! Confidence assessment for cleanup candidates.
//!
//! The scanner answers "is this a build artifact?". This module answers a
//! different question: "how sure are we?". Keeping the two apart is what lets
//! the product search aggressively and delete conservatively.
//!
//! Assessment is a pure function of collected signals. It performs no I/O, so
//! the code that decides whether something may be deleted can be tested
//! exhaustively without a filesystem, and behaves identically on every caller.

use serde::{Deserialize, Serialize};

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

const THRESHOLD_HIGH: i32 = 80;
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
    fn recent_writes_pull_a_candidate_down_a_level() {
        let settled = assess(ConfidenceInput {
            git_ignored: Some(false),
            ..project_artifact()
        });
        let recent = assess(ConfidenceInput {
            git_ignored: Some(false),
            recently_modified: true,
            ..project_artifact()
        });

        assert_eq!(settled.level, ConfidenceLevel::High);
        assert_eq!(recent.level, ConfidenceLevel::Medium);
        assert!(recent.signals.contains(&ConfidenceSignal::RecentlyModified));
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

    #[test]
    fn assessment_is_deterministic() {
        let input = project_artifact();
        assert_eq!(assess(input), assess(input));
    }
}
