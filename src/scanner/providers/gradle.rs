use crate::model::Category;
use crate::scanner::Scanner;
use std::path::Path;

pub struct GradleScanner;

impl Scanner for GradleScanner {
    fn category(&self) -> Category {
        Category::Gradle
    }
    fn matches(&self, name: &str) -> bool {
        name == ".gradle"
    }
    /// Context-free by design: `.gradle` is a tool-generated cache and is
    /// reported wherever it is reachable inside the scanned workspace.
    fn confirm_context(&self, _path: &Path) -> bool {
        true
    }
}
