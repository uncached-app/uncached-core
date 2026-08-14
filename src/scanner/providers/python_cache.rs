use crate::model::Category;
use crate::scanner::Scanner;
use std::path::Path;

pub struct PyCacheScanner;

impl Scanner for PyCacheScanner {
    fn category(&self) -> Category {
        Category::PyCache
    }
    fn matches(&self, name: &str) -> bool {
        matches!(
            name,
            "__pycache__"
                | ".pytest_cache"
                | ".mypy_cache"
                | ".ruff_cache"
                | ".tox"
                | ".nox"
                | "__pypackages__"
        )
    }
    /// Context-free by design: these directories are tool-generated caches whose
    /// name alone identifies them, with no marker file to confirm against.
    ///
    /// Virtual environments are deliberately not here. `venv` and `env` are
    /// ordinary directory names, and an environment costs network to rebuild,
    /// which puts it in a different class from a cache.
    fn confirm_context(&self, _path: &Path) -> bool {
        true
    }
}
