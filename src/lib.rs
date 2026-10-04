pub mod cleaner;
pub mod confidence;
pub mod contract;
pub mod dependencies;
pub mod discovery;
pub mod error;
pub mod filesystem;
pub mod ignore_rules;
pub mod model;
pub mod scanner;

mod parallel;

mod secure_cleanup;

pub use cleaner::{clean_permanently, clean_to_trash, validate_clean_target};
pub use confidence::{
    assess, collect_signals, Confidence, ConfidenceInput, ConfidenceLevel, ConfidenceSignal,
};
pub use dependencies::{
    analyse_manifest, analyse_project, DependencyKind, DependencyReport, Ecosystem, ManifestError,
    ProjectDependencies, UnusedDependency, UnusedReason,
};
pub use discovery::{
    home_directory, known_locations, probe_home, probe_system, probe_temp, Discovery,
    KnownLocation, KnownLocationScanner,
};
pub use error::{CoreError, CoreErrorCode, CoreErrorInfo, CoreResult};
pub use filesystem::canonical;
pub use ignore_rules::{IgnoreRuleError, IgnoreRules};
pub use model::{Category, CleanReport, FileIdentity, LocationKind, ScanTarget, TraversalOutcome};
pub use scanner::{
    ecosystems, traverse_workspace, CargoTargetScanner, DartToolScanner, DotNetObjScanner,
    GradleScanner, NextCacheScanner, NodeModulesScanner, ProjectMarker, ProjectScanner,
    ProviderSet, PyCacheScanner, Scanner, SwiftBuildScanner, TraversalControl, TraversalEntry,
    TurboCacheScanner, XcodeDerivedDataScanner,
};

/// The cleanup backends this platform offers, in the shape the app receives.
///
/// The mapping lives here, at the edge, so the cleanup layer reports facts in
/// its own terms and never depends on the wire contract.
pub fn cleanup_capabilities() -> contract::v1::CleanupCapabilities {
    let backend = secure_cleanup::backend_capabilities();
    contract::v1::CleanupCapabilities {
        permanent_cleanup: backend.permanent,
        move_to_trash: backend.trash,
        secure_identity: backend.secure_identity,
    }
}
