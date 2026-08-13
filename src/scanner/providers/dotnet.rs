use crate::filesystem::has_marker_with_extension;
use crate::model::Category;
use crate::scanner::Scanner;
use std::path::Path;

/// .NET's intermediate and output directories.
///
/// `bin` and `obj` are among the most reused directory names there are, so the
/// project file beside them is not corroboration here — it is the entire
/// reason the match is allowed. Without it this provider would offer to delete
/// the `bin` of anything.
pub struct DotNetObjScanner;

const PROJECT_EXTENSIONS: &[&str] = &["csproj", "fsproj", "vbproj", "sln"];

impl Scanner for DotNetObjScanner {
    fn category(&self) -> Category {
        Category::DotNetObj
    }

    fn matches(&self, name: &str) -> bool {
        name == "obj" || name == "bin"
    }

    fn confirm_context(&self, path: &Path) -> bool {
        path.parent().is_some_and(|parent| {
            PROJECT_EXTENSIONS
                .iter()
                .any(|extension| has_marker_with_extension(parent, extension))
        })
    }
}

/// Dart and Flutter's package resolution and build state.
pub struct DartToolScanner;

impl Scanner for DartToolScanner {
    fn category(&self) -> Category {
        Category::DartTool
    }

    fn matches(&self, name: &str) -> bool {
        name == ".dart_tool"
    }

    fn confirm_context(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| crate::filesystem::has_marker_file(parent, "pubspec.yaml"))
    }
}
