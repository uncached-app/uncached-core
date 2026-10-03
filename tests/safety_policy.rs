//! What the crate treats as a protected system location, through its public API.
//!
//! Three questions are answered from one table (`filesystem::protected`): may a
//! directory be deleted, may it be a scan root, and does a candidate beneath it
//! lose its confidence. They are different questions, so the answers may differ
//! — but only on purpose, and the rows below are those purposes:
//!
//! - the deletion gate refuses the protected directory *itself* on Unix, and the
//!   whole tree on Windows; what is beneath on Unix is left to workspace
//!   containment and the providers;
//! - `/var` is a scan root nobody wants but is not demoted, because macOS keeps
//!   per-user temp directories under `/private/var`;
//! - `/opt` and `/home` stay usable as scan roots.
//!
//! The table's own invariants are tested beside it. These tests are the outside
//! view: they would catch a caller that stopped asking the table.
//!
//! Everything here is pure path reasoning except the workspace check, which
//! needs the directory to exist, so that part sticks to ones every Linux has.

use std::path::Path;
use uncached_core::collect_signals;

fn marked_as_system(path: &str) -> bool {
    collect_signals(Path::new(path), false).system_path
}

#[cfg(unix)]
mod unix_deletion_gate {
    use super::*;
    use uncached_core::filesystem::is_safe_to_delete;

    #[test]
    fn the_protected_directories_themselves_are_refused() {
        for path in [
            "/usr",
            "/bin",
            "/sbin",
            "/etc",
            "/var",
            "/home",
            "/root",
            "/opt",
            "/lib",
            "/lib64",
            "/boot",
            "/dev",
            "/proc",
            "/sys",
            "/run",
            "/System",
            "/Library",
            "/Applications",
        ] {
            assert!(!is_safe_to_delete(Path::new(path)), "{path}");
        }
        assert!(
            !is_safe_to_delete(Path::new("/")),
            "the root has no parent to delete from"
        );
    }

    /// The gate compares for equality, so everything *under* a protected
    /// directory passes it. Nothing else stops a deletion there except the
    /// workspace containment and the provider's own check.
    #[test]
    fn what_is_beneath_a_protected_directory_passes_the_gate_on_unix() {
        for path in [
            "/usr/local/lib/node_modules",
            "/opt/app/node_modules",
            "/var/tmp/project/target",
            "/home/someone/project/node_modules",
        ] {
            assert!(is_safe_to_delete(Path::new(path)), "{path}");
        }
    }
}

mod confidence_veto {
    use super::*;

    #[test]
    fn a_path_under_a_system_root_is_marked_but_the_root_itself_is_not() {
        assert!(marked_as_system("/usr/lib/node_modules"));
        assert!(marked_as_system("/opt/app/node_modules"));
        assert!(marked_as_system("/Library/Caches/thing"));
        assert!(!marked_as_system("/usr"), "only what is under the root");
        assert!(!marked_as_system("/opt"));
    }

    /// `/var` is left out of the veto on purpose: macOS keeps per-user
    /// temporary directories under `/private/var`. The other two lists do
    /// include it.
    #[test]
    fn var_is_not_a_system_path_to_the_veto() {
        assert!(!marked_as_system("/var/tmp/project/target"));
    }

    #[test]
    fn every_operating_system_root_demotes_what_is_beneath_it() {
        for path in [
            "/lib/x", "/lib64/x", "/boot/x", "/dev/x", "/proc/x", "/run/x", "/sys/x",
        ] {
            assert!(marked_as_system(path), "{path}");
        }
    }

    /// A home directory is the user's own, root's included.
    #[test]
    fn a_home_directory_does_not_demote() {
        for path in ["/root/x", "/home/someone/x"] {
            assert!(!marked_as_system(path), "{path}");
        }
    }

    #[test]
    fn ordinary_home_and_temp_paths_are_not_system_paths() {
        assert!(!marked_as_system("/home/someone/project/node_modules"));
        assert!(!marked_as_system("/tmp/project/node_modules"));
    }
}

/// Where the questions differ on purpose, side by side.
#[cfg(unix)]
mod disagreements {
    use super::*;
    use uncached_core::filesystem::is_safe_to_delete;

    #[test]
    fn usr_local_beneath_the_root_is_deletable_by_the_gate_yet_demoted() {
        let path = "/usr/local/lib/node_modules";
        assert!(is_safe_to_delete(Path::new(path)));
        assert!(marked_as_system(path));
    }

    #[test]
    fn var_is_refused_by_the_gate_yet_not_demoted() {
        assert!(!is_safe_to_delete(Path::new("/var")));
        assert!(!marked_as_system("/var/x"));
    }
}

/// The workspace rule is enforced on Linux and macOS, and on Windows below.
/// Existing system directories are used so the check goes through the real
/// canonicalisation; on macOS that is what turns `/etc` into `/private/etc`.
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_workspace_rule {
    use std::path::Path;
    use uncached_core::filesystem;

    #[test]
    fn system_roots_are_not_accepted_as_a_workspace() {
        for path in ["/", "/usr", "/etc", "/var", "/bin", "/sbin"] {
            if Path::new(path).is_dir() {
                assert!(
                    filesystem::canonical_workspace(Path::new(path)).is_err(),
                    "{path}"
                );
            }
        }
    }

    /// Neither is a place the operating system owns the contents of; both are
    /// ordinary places to keep a project.
    #[test]
    fn opt_and_home_are_accepted_as_a_workspace() {
        for path in ["/opt", "/home"] {
            if Path::new(path).is_dir() {
                assert!(
                    filesystem::canonical_workspace(Path::new(path)).is_ok(),
                    "{path} is accepted as a workspace today"
                );
            }
        }
    }

    #[test]
    fn a_project_directory_is_accepted() {
        let root = std::env::temp_dir().join(format!("uncached-policy-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(filesystem::canonical_workspace(&root).is_ok());
        std::fs::remove_dir_all(&root).unwrap();
    }
}

/// Windows has no `/usr`; its system directories come from the environment, and
/// a project directory does not live in them.
#[cfg(windows)]
mod windows_workspace_rule {
    use std::path::PathBuf;
    use uncached_core::filesystem;

    fn from_env(name: &str) -> Option<PathBuf> {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
    }

    #[test]
    fn the_windows_and_program_files_trees_are_not_accepted_as_a_workspace() {
        for name in ["SystemRoot", "ProgramFiles"] {
            let Some(dir) = from_env(name) else { continue };
            assert!(filesystem::canonical_workspace(&dir).is_err(), "{name}");
            let below = dir.join("System32");
            if below.is_dir() {
                assert!(filesystem::canonical_workspace(&below).is_err(), "{name}");
            }
        }
    }

    #[test]
    fn a_project_directory_is_accepted() {
        let root = std::env::temp_dir().join(format!("uncached-policy-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let accepted = filesystem::canonical_workspace(&root).is_ok();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(accepted);
    }
}
