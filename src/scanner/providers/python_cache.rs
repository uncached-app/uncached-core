use super::super::Scanner;
use crate::model::Category;
use std::path::Path;

pub struct PyCacheScanner;

impl Scanner for PyCacheScanner {
    fn category(&self) -> Category {
        Category::PyCache
    }
    fn matches(&self, name: &str) -> bool {
        matches!(name, "__pycache__" | ".pytest_cache" | ".mypy_cache")
    }
    fn confirm_context(&self, _path: &Path) -> bool {
        true
    }
}
