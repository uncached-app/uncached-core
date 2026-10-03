//! The one table of locations the operating system owns.
//!
//! Three questions used to be answered by three lists written for three
//! callers, which disagreed with each other and recorded nowhere why:
//!
//! - may this directory be deleted at all (the last gate before a removal),
//! - may it be a scan root,
//! - should a candidate under it be dropped to `Review`.
//!
//! They are different questions, so the answers may differ. What they may not
//! do is differ by accident. Each root below states, per question, how far the
//! protection reaches — and a root that appears in this table can never be
//! deleted itself, which is the one rule every row shares.
//!
//! The reach is also where the platforms genuinely differ, and the table says
//! so rather than hiding it behind `cfg` inside a function: on Unix the
//! deletion gate refuses the directory itself and relies on workspace
//! containment and the providers for what is beneath it; on Windows it refuses
//! the whole tree.

use std::path::Path;

/// How far a protection reaches from the directory it names.
#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Only the directory itself.
    Itself,
    /// Only what is strictly beneath it.
    Beneath,
    /// The directory and everything beneath it.
    Within,
}

#[cfg(not(windows))]
impl Reach {
    fn covers(self, root: &Path, path: &Path) -> bool {
        match self {
            Self::Itself => path == root,
            Self::Beneath => path != root && path.starts_with(root),
            Self::Within => path.starts_with(root),
        }
    }
}

/// One protected location and what each question says about it.
#[cfg(not(windows))]
struct Root {
    path: &'static str,
    /// May it be deleted. Always `Some`: nothing in this table may be.
    deletion: Option<Reach>,
    /// May it be a scan root.
    workspace: Option<Reach>,
    /// Does a candidate beneath it lose its confidence.
    confidence: Option<Reach>,
}

#[cfg(not(windows))]
const fn root(path: &'static str, workspace: Option<Reach>, confidence: Option<Reach>) -> Root {
    Root {
        path,
        deletion: Some(Reach::Itself),
        workspace,
        confidence,
    }
}

/// Whether the workspace rule is enforced on this platform.
///
/// Only Linux, as it always was. On macOS and Windows any existing directory is
/// accepted as a scan root, whatever it is: a known gap, kept visible here so it
/// is a decision somebody makes rather than an `if` somebody forgets.
#[cfg(not(windows))]
const WORKSPACE_RULE_ENFORCED: bool = cfg!(target_os = "linux");

#[cfg(not(windows))]
const W: Option<Reach> = Some(Reach::Within);
#[cfg(not(windows))]
const B: Option<Reach> = Some(Reach::Beneath);
#[cfg(not(windows))]
const NO: Option<Reach> = None;

#[cfg(not(windows))]
static ROOTS: &[Root] = &[
    // The filesystem root: a scan of it is a scan of the machine. Nothing is
    // demoted beneath it — that would demote every path there is — and it has
    // no parent, so the deletion gate refuses it on that ground as well.
    root("/", Some(Reach::Itself), NO),
    // What the operating system itself is made of.
    root("/usr", W, B),
    root("/bin", W, B),
    root("/sbin", W, B),
    root("/etc", W, B),
    root("/lib", W, B),
    root("/lib64", W, B),
    root("/boot", W, B),
    root("/dev", W, B),
    root("/proc", W, B),
    root("/sys", W, B),
    root("/run", W, B),
    // macOS system locations. Nobody scans these as a workspace and nothing a
    // project builds lives under them.
    root("/System", NO, B),
    root("/Library", NO, B),
    root("/Applications", NO, B),
    // `/opt` holds installed software, so what is under it is demoted, yet it
    // is an ordinary place to keep a project and stays usable as a scan root.
    root("/opt", NO, B),
    // `/var` is a scan root nobody wants and a directory nobody may delete, but
    // it is deliberately *not* demoted: macOS keeps per-user temporary
    // directories under `/private/var`, so treating it as a system path would
    // mislabel ordinary scratch space.
    root("/var", W, NO),
    // Home directories. The directory holding them is never deleted, and what
    // is in them is the user's own.
    root("/home", NO, NO),
    root("/root", W, NO),
];

#[cfg(not(windows))]
fn protects(concern: fn(&Root) -> Option<Reach>, path: &Path) -> bool {
    ROOTS
        .iter()
        .any(|entry| concern(entry).is_some_and(|reach| reach.covers(Path::new(entry.path), path)))
}

/// Whether `path` may not be deleted. Callers pass a canonical path.
#[cfg(not(windows))]
pub(crate) fn refuses_deletion(path: &Path) -> bool {
    protects(|entry| entry.deletion, path)
}

/// Whether `path` may not be used as a scan root.
#[cfg(not(windows))]
pub(crate) fn refuses_workspace(path: &Path) -> bool {
    WORKSPACE_RULE_ENFORCED && protects(|entry| entry.workspace, path)
}

/// Whether a candidate at `path` loses its confidence for sitting in a place the
/// operating system owns.
#[cfg(not(windows))]
pub(crate) fn demotes_confidence(path: &Path) -> bool {
    protects(|entry| entry.confidence, path)
}

/// On Windows the roots come from the environment, so the table is built when
/// asked. The deletion gate refuses the whole tree; the scan-root rule is not
/// enforced (see `WORKSPACE_RULE_ENFORCED` for the other platforms).
#[cfg(windows)]
fn roots() -> [String; 3] {
    [
        std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string()),
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string()),
        std::env::var("ProgramFiles(x86)")
            .unwrap_or_else(|_| r"C:\Program Files (x86)".to_string()),
    ]
    .map(|value| value.trim_end_matches(['\\', '/']).to_lowercase())
}

#[cfg(windows)]
pub(crate) fn refuses_deletion(path: &Path) -> bool {
    let normalized = path.to_string_lossy().to_lowercase();
    roots()
        .iter()
        .any(|root| normalized == *root || normalized.starts_with(&format!("{root}\\")))
}

#[cfg(windows)]
pub(crate) fn refuses_workspace(_path: &Path) -> bool {
    false
}

#[cfg(windows)]
pub(crate) fn demotes_confidence(path: &Path) -> bool {
    let normalized = path.to_string_lossy().to_lowercase();
    roots()
        .iter()
        .any(|root| normalized.starts_with(&format!("{root}\\")))
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    /// The rule every row shares. If a root could be deleted, listing it here
    /// would be a lie.
    #[test]
    fn every_protected_root_refuses_its_own_deletion() {
        for entry in ROOTS {
            assert_eq!(entry.deletion, Some(Reach::Itself), "{}", entry.path);
            assert!(refuses_deletion(Path::new(entry.path)), "{}", entry.path);
        }
    }

    /// A directory refused as a scan root while anyone may still delete it would
    /// be a contradiction between two questions, not a difference of purpose.
    #[test]
    fn a_root_refused_as_a_workspace_cannot_be_deleted() {
        for entry in ROOTS.iter().filter(|entry| entry.workspace.is_some()) {
            assert!(refuses_deletion(Path::new(entry.path)), "{}", entry.path);
        }
    }

    #[test]
    fn rows_are_absolute_unique_and_have_no_trailing_slash() {
        let mut seen = std::collections::HashSet::new();
        for entry in ROOTS {
            assert!(entry.path.starts_with('/'), "{}", entry.path);
            assert!(
                entry.path == "/" || !entry.path.ends_with('/'),
                "{}",
                entry.path
            );
            assert!(seen.insert(entry.path), "{} appears twice", entry.path);
        }
    }

    #[test]
    fn reach_covers_exactly_what_it_says() {
        let root = Path::new("/usr");
        let beneath = Path::new("/usr/local/lib");
        let sibling = Path::new("/usr-local");

        assert!(Reach::Itself.covers(root, root));
        assert!(!Reach::Itself.covers(root, beneath));
        assert!(!Reach::Beneath.covers(root, root));
        assert!(Reach::Beneath.covers(root, beneath));
        assert!(Reach::Within.covers(root, root));
        assert!(Reach::Within.covers(root, beneath));
        for reach in [Reach::Itself, Reach::Beneath, Reach::Within] {
            assert!(
                !reach.covers(root, sibling),
                "{reach:?}: a name is not a path"
            );
        }
    }

    #[test]
    fn deletion_refuses_the_root_and_not_what_is_beneath_it() {
        assert!(refuses_deletion(Path::new("/usr")));
        assert!(!refuses_deletion(Path::new("/usr/local/lib/node_modules")));
        assert!(!refuses_deletion(Path::new("/home/someone/project")));
    }

    #[test]
    fn confidence_demotes_what_is_beneath_a_root_and_not_the_root() {
        assert!(demotes_confidence(Path::new("/usr/lib/node_modules")));
        assert!(demotes_confidence(Path::new("/run/thing/target")));
        assert!(!demotes_confidence(Path::new("/usr")));
        assert!(!demotes_confidence(Path::new("/var/tmp/project/target")));
        assert!(!demotes_confidence(Path::new("/home/someone/node_modules")));
        assert!(
            !demotes_confidence(Path::new("/")),
            "the filesystem root must not demote every path"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_workspace_rule_refuses_system_roots_and_not_projects() {
        for path in [
            "/",
            "/usr",
            "/usr/local",
            "/etc",
            "/bin",
            "/lib64",
            "/var",
            "/root",
        ] {
            assert!(refuses_workspace(Path::new(path)), "{path}");
        }
        for path in ["/opt", "/home", "/home/someone/project", "/tmp/project"] {
            assert!(!refuses_workspace(Path::new(path)), "{path}");
        }
    }
}
