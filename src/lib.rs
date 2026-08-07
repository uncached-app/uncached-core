pub mod cleaner;
pub mod error;
pub mod filesystem;
pub mod model;
pub mod scanner;

pub use cleaner::{clean_to_trash, clean_to_trash_with_delete, validate_clean_target};
pub use error::{CoreError, CoreResult};
pub use model::{Category, CleanReport, ScanReport, ScanTarget};
pub use scanner::{scan_workspace, NodeModulesScanner, ProviderSet, Scanner};
