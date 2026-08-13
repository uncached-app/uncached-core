pub mod cleaner;
pub mod confidence;
pub mod contract;
pub mod error;
pub mod filesystem;
pub mod model;
pub mod scanner;

mod parallel;

mod secure_cleanup;

pub use cleaner::{clean_permanently, clean_to_trash, validate_clean_target};
pub use confidence::{
    assess, collect_signals, Confidence, ConfidenceInput, ConfidenceLevel, ConfidenceSignal,
};
pub use error::{CoreError, CoreErrorCode, CoreErrorInfo, CoreResult};
pub use model::{Category, CleanReport, FileIdentity, ScanTarget, TraversalOutcome};
pub use scanner::{
    traverse_workspace, CargoTargetScanner, GradleScanner, NextCacheScanner, NodeModulesScanner,
    ProviderSet, PyCacheScanner, Scanner, TraversalControl, TraversalEntry, TurboCacheScanner,
};

pub fn cleanup_capabilities() -> contract::v1::CleanupCapabilities {
    secure_cleanup::cleanup_capabilities()
}
