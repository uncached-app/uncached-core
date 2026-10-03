//! What the crate currently treats as a protected system location.
//!
//! The answer is not in one place. Three independent lists decide it, each
//! written for its own caller, and they were never reconciled:
//!
//! - `is_safe_to_delete`: the last gate before a deletion. Unix compares the
//!   path for *equality* with a short list, so only the directory itself is
//!   refused, not what is under it.
//! - `canonical_workspace`: which scan roots are accepted at all. On Linux only.
//! - `confidence::collect_signals(..).system_path`: the veto that drops a
//!   candidate to `Review`.
//!
//! These tests pin that behaviour as it is today, including the places the
//! lists disagree, so that reconciling them later shows up as a deliberate
//! change to a named expectation rather than as a side effect. A test named
//! `current_*` documents a fact about the code, not a requirement: where the
//! two sides of a row differ, one of them is probably wrong.
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
    fn current_the_critical_directories_themselves_are_refused() {
        for path in [
            "/usr", "/bin", "/sbin", "/etc", "/var", "/home", "/root", "/opt",
        ] {
            assert!(!is_safe_to_delete(Path::new(path)), "{path}");
        }
        assert!(
            !is_safe_to_delete(Path::new("/")),
            "the root has no parent to delete from"
        );
    }

    /// The gate compares for equality, so everything *under* a critical
    /// directory passes it. Nothing else stops a deletion there except the
    /// workspace containment and the provider's own check.
    #[test]
    fn current_anything_under_a_critical_directory_passes_the_gate() {
        for path in [
            "/usr/local/lib/node_modules",
            "/opt/app/node_modules",
            "/var/tmp/project/target",
            "/home/someone/project/node_modules",
        ] {
            assert!(is_safe_to_delete(Path::new(path)), "{path}");
        }
    }

    /// Directories the other two lists protect and this one does not name.
    #[test]
    fn current_the_gate_does_not_name_every_directory_the_workspace_rule_does() {
        for path in ["/lib", "/lib64", "/boot", "/dev", "/proc", "/sys", "/run"] {
            assert!(is_safe_to_delete(Path::new(path)), "{path}");
        }
    }
}

mod confidence_veto {
    use super::*;

    #[test]
    fn current_a_path_under_a_system_root_is_marked_but_the_root_itself_is_not() {
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
    fn current_var_is_not_a_system_path_to_the_veto() {
        assert!(!marked_as_system("/var/tmp/project/target"));
    }

    #[test]
    fn current_the_veto_does_not_name_every_directory_the_workspace_rule_does() {
        for path in ["/lib/x", "/boot/x", "/root/x", "/run/x", "/sys/x"] {
            assert!(!marked_as_system(path), "{path}");
        }
    }

    #[test]
    fn current_ordinary_home_and_temp_paths_are_not_system_paths() {
        assert!(!marked_as_system("/home/someone/project/node_modules"));
        assert!(!marked_as_system("/tmp/project/node_modules"));
    }
}

/// The two paths where the lists disagree about the same directory, side by
/// side. If these ever agree, the lists have been reconciled.
#[cfg(unix)]
mod disagreements {
    use super::*;
    use uncached_core::filesystem::is_safe_to_delete;

    #[test]
    fn current_usr_local_is_deletable_yet_marked_as_system() {
        let path = "/usr/local/lib/node_modules";
        assert!(is_safe_to_delete(Path::new(path)));
        assert!(marked_as_system(path));
    }

    #[test]
    fn current_var_is_refused_by_the_gate_yet_not_marked_as_system() {
        assert!(!is_safe_to_delete(Path::new("/var")));
        assert!(!marked_as_system("/var/x"));
    }
}

/// The workspace rule is only enforced on Linux. On every other platform any
/// existing directory is accepted as a scan root, whatever it is.
#[cfg(target_os = "linux")]
mod linux_workspace_rule {
    use std::path::Path;
    use uncached_core::filesystem;

    #[test]
    fn current_system_roots_are_not_accepted_as_a_workspace() {
        for path in ["/", "/usr", "/etc", "/var"] {
            if Path::new(path).is_dir() {
                assert!(
                    filesystem::canonical_workspace(Path::new(path)).is_err(),
                    "{path}"
                );
            }
        }
    }

    /// `/opt` is in the other two lists and `/home` in the deletion gate, but
    /// neither is in this one, so both are accepted as a scan root.
    #[test]
    fn current_the_workspace_list_omits_opt_and_home() {
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
    fn current_a_project_directory_is_accepted() {
        let root = std::env::temp_dir().join(format!("uncached-policy-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(filesystem::canonical_workspace(&root).is_ok());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
