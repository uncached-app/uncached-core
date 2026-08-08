pub mod cleaner;
pub mod error;
pub mod filesystem;
pub mod model;
pub mod scanner;

#[allow(dead_code)]
mod secure_cleanup;

pub use cleaner::{
    clean_permanently, clean_permanently_with_delete_and_size, clean_to_trash,
    clean_to_trash_with_delete, clean_to_trash_with_delete_and_size, validate_clean_target,
};
pub use error::{CoreError, CoreErrorCode, CoreErrorInfo, CoreResult};
pub use model::{
    Category, CleanReport, FileIdentity, ScanOutcome, ScanReport, ScanTarget, TraversalOutcome,
};
pub use scanner::{
    traverse_workspace, CargoTargetScanner, GradleScanner, NextCacheScanner, NodeModulesScanner,
    ProviderSet, PyCacheScanner, Scanner, TraversalControl, TraversalEntry, TurboCacheScanner,
};
