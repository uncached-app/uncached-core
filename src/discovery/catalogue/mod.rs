//! The table of locations discovery knows by name, and the shape of one entry.
//!
//! One file per platform, because the three tables share nothing but this
//! shape: a path is under `Library` on macOS, `~/.cache` on Linux and
//! `AppData` on Windows, and an entry earns its place per platform.

use crate::model::{Category, LocationKind};
use std::path::{Path, PathBuf};

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

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::PLATFORM_LOCATIONS;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::PLATFORM_LOCATIONS;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::PLATFORM_LOCATIONS;

/// The table, per platform.
pub fn known_locations() -> &'static [KnownLocation] {
    PLATFORM_LOCATIONS
}
