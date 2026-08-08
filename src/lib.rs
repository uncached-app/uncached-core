pub mod cleaner;
pub mod error;
pub mod filesystem;
pub mod model;
pub mod scanner;

pub use cleaner::{
    clean_to_trash, clean_to_trash_with_delete, clean_to_trash_with_delete_and_size,
    validate_clean_target,
};
pub use error::{CoreError, CoreResult};
pub use model::{
    Category, CleanReport, FileIdentity, ScanOutcome, ScanReport, ScanTarget, TraversalOutcome,
};
pub use scanner::{
    scan_workspace, scan_workspace_with_sizing, traverse_workspace, NodeModulesScanner,
    ProviderSet, Scanner, TraversalControl, TraversalEntry,
};
