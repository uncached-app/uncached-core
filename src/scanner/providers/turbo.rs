use super::super::Scanner;
use crate::model::Category;
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
            parent.join("package.json").exists() || parent.join("turbo.json").exists()
        })
    }
}
