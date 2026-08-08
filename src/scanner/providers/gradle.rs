use super::super::Scanner;
use crate::model::Category;
use std::path::Path;

pub struct GradleScanner;

impl Scanner for GradleScanner {
    fn category(&self) -> Category {
        Category::Gradle
    }
    fn matches(&self, name: &str) -> bool {
        name == ".gradle"
    }
    fn confirm_context(&self, _path: &Path) -> bool {
        true
    }
}
