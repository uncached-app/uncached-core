use crate::filesystem::has_marker_file;
use crate::model::Category;
use crate::scanner::Scanner;
use std::path::Path;

pub struct CargoTargetScanner;

impl Scanner for CargoTargetScanner {
    fn category(&self) -> Category {
        Category::CargoTarget
    }
    /// `.xwin-cache` holds the Windows SDK a cross-compiling build downloads,
    /// and it sits beside `target` in the same crate.
    fn matches(&self, name: &str) -> bool {
        name == "target" || name == ".xwin-cache"
    }
    fn confirm_context(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| has_marker_file(parent, "Cargo.toml"))
    }
}
