use crate::model::Category;
use crate::scanner::Scanner;
use std::path::Path;

pub struct PyCacheScanner;

impl Scanner for PyCacheScanner {
    fn category(&self) -> Category {
        Category::PyCache
    }
    fn matches(&self, name: &str) -> bool {
        matches!(name, "__pycache__" | ".pytest_cache" | ".mypy_cache")
    }
    /// Context-free by design: these directories are tool-generated caches whose
    /// name alone identifies them, with no marker file to confirm against.
    fn confirm_context(&self, _path: &Path) -> bool {
        true
    }
}
