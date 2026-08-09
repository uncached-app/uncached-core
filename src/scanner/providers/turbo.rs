use crate::filesystem::has_marker_file;
use crate::model::Category;
use crate::scanner::Scanner;
use std::path::Path;

pub struct TurboCacheScanner;

impl Scanner for TurboCacheScanner {
    fn category(&self) -> Category {
        Category::Turbo
    }
    fn matches(&self, name: &str) -> bool {
        name == ".turbo"
    }
    fn confirm_context(&self, path: &Path) -> bool {
        path.parent().is_some_and(|parent| {
            has_marker_file(parent, "package.json") || has_marker_file(parent, "turbo.json")
        })
    }
}
