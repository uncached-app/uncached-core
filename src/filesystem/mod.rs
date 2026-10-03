//! The crate's view of the filesystem, one concern per file.
//!
//! - [`path`]: the single canonical spelling of a path, and which workspace
//!   roots are acceptable at all.
//! - [`protected`]: the one table of locations the operating system owns, and
//!   how far each protection reaches.
//! - [`safety`]: the deletion gate and containment within a root.
//! - [`identity`]: the stable identity a target is pinned by between scan and
//!   cleanup.
//! - [`sizing`]: how much space a tree holds and how recently it was written.
//! - [`markers`]: the cheap checks providers use to confirm a project.
//!
//! Everything is re-exported here, so callers keep using `filesystem::name`.

mod identity;
mod markers;
mod path;
mod protected;
mod safety;
mod sizing;

pub use identity::file_identity;
pub use markers::{
    has_any_marker_file, has_marker_directory, has_marker_file, has_marker_with_extension,
};
pub use path::{canonical, canonical_workspace};
pub(crate) use protected::demotes_confidence;
pub use safety::{is_protected_directory, is_safe_to_delete, is_within};
pub use sizing::{
    directory_size_report, directory_size_report_skipping, directory_size_report_with_cancel,
    DirectorySizeReport,
};
