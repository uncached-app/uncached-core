use crate::filesystem::has_marker_file;
use crate::model::Category;
use crate::scanner::Scanner;
use std::path::Path;

pub struct NextCacheScanner;

impl Scanner for NextCacheScanner {
    fn category(&self) -> Category {
        Category::NextJs
    }
    fn matches(&self, name: &str) -> bool {
        name == ".next"
    }
    fn confirm_context(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| has_marker_file(parent, "package.json"))
    }
}
