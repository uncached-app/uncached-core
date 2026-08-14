use crate::filesystem::{has_marker_file, has_marker_with_extension};
use crate::model::Category;
use crate::scanner::Scanner;
use std::path::Path;

/// Xcode's build products for a project configured to keep them beside the
/// sources rather than in the shared `~/Library/Developer/Xcode/DerivedData`.
///
/// The shared location holds far more, but it is not reachable from a workspace
/// walk: it is found by [`crate::discovery`] instead. Both report under the same
/// category, because they hold the same thing.
pub struct XcodeDerivedDataScanner;

impl Scanner for XcodeDerivedDataScanner {
    fn category(&self) -> Category {
        Category::XcodeDerivedData
    }

    fn matches(&self, name: &str) -> bool {
        name == "DerivedData" || name == "build"
    }

    /// Requires an Xcode project or workspace beside it. `build` is far too
    /// common a directory name to accept on its own, and a `DerivedData`
    /// somewhere else is not necessarily Xcode's.
    fn confirm_context(&self, path: &Path) -> bool {
        path.parent().is_some_and(is_xcode_project_root)
    }
}

/// Swift Package Manager's build directory.
pub struct SwiftBuildScanner;

impl Scanner for SwiftBuildScanner {
    fn category(&self) -> Category {
        Category::SwiftBuild
    }

    fn matches(&self, name: &str) -> bool {
        name == ".build" || name == ".swiftpm"
    }

    fn confirm_context(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| has_marker_file(parent, "Package.swift"))
    }
}

fn is_xcode_project_root(parent: &Path) -> bool {
    has_marker_with_extension(parent, "xcodeproj")
        || has_marker_with_extension(parent, "xcworkspace")
}
