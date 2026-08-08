use super::super::Scanner;
use crate::model::Category;
use std::path::Path;

pub struct CargoTargetScanner;

impl Scanner for CargoTargetScanner {
    fn category(&self) -> Category {
        Category::CargoTarget
    }
    fn matches(&self, name: &str) -> bool {
        name == "target"
    }
    fn confirm_context(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| parent.join("Cargo.toml").exists())
    }
}
