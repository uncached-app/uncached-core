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

mod catalogue;
mod probe;
mod provider;

pub use crate::model::LocationKind;
pub use catalogue::{known_locations, KnownLocation};
pub use probe::{home_directory, probe_home, probe_system, probe_temp, signals_for, Discovery};
pub use provider::KnownLocationScanner;

#[cfg(test)]
use {
    probe::{probe_absolute, SystemLocation},
    provider::last_segment,
};
// Unix-only, like the function: there is no session scratch to recognise elsewhere.
#[cfg(all(test, unix))]
use probe::is_session_scratch;

#[cfg(test)]
mod tests;
