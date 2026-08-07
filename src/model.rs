use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    NodeModules,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanTarget {
    pub path: PathBuf,
    pub category: Category,
    pub size_bytes: u64,
}

impl ScanTarget {
    pub fn new(path: PathBuf, category: Category, size_bytes: u64) -> Self {
        Self {
            path,
            category,
            size_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    pub targets: Vec<ScanTarget>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanReport {
    pub freed_bytes: u64,
}
