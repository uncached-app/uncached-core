use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalOutcome {
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    Completed(ScanReport),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    NodeModules,
    CargoTarget,
    NextJs,
    Turbo,
    PyCache,
    Gradle,
    DotNetObj,
    Docker,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanTarget {
    pub path: PathBuf,
    pub category: Category,
    pub size_bytes: u64,
    pub identity: Option<FileIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub volume_serial: u64,
    pub file_id: [u8; 16],
}

#[cfg(test)]
mod tests {
    use super::FileIdentity;

    #[test]
    fn file_identity_round_trips_through_json() {
        let identity = FileIdentity {
            volume_serial: 42,
            file_id: [7; 16],
        };

        let encoded = serde_json::to_string(&identity).expect("identity should serialize");
        let decoded: FileIdentity =
            serde_json::from_str(&encoded).expect("identity should deserialize");

        assert_eq!(decoded, identity);
    }

    #[test]
    fn file_identity_detects_volume_and_file_id_mismatches() {
        let identity = FileIdentity {
            volume_serial: 42,
            file_id: [7; 16],
        };

        assert_ne!(
            identity,
            FileIdentity {
                volume_serial: 43,
                file_id: [7; 16],
            }
        );
        assert_ne!(
            identity,
            FileIdentity {
                volume_serial: 42,
                file_id: [8; 16],
            }
        );
    }
}

impl ScanTarget {
    pub fn new(path: PathBuf, category: Category, size_bytes: u64) -> Self {
        Self {
            path,
            category,
            size_bytes,
            identity: None,
        }
    }

    pub fn with_identity(mut self, identity: Option<FileIdentity>) -> Self {
        self.identity = identity;
        self
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
    pub size_has_errors: bool,
}
