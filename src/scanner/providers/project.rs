use crate::filesystem::{has_marker_file, has_marker_with_extension};
use crate::model::Category;
use crate::scanner::Scanner;
use std::path::Path;

/// What identifies the project a set of artifacts belongs to.
pub enum ProjectMarker {
    /// Any of these file names sits directly in the project root.
    AnyFile(&'static [&'static str]),
    /// Any file carrying one of these extensions sits in the project root.
    /// Some ecosystems name the project file after the project rather than
    /// after the tool, so there is no fixed name to look for.
    AnyExtension(&'static [&'static str]),
    /// For the roots that take more than a file to recognize.
    Custom(fn(&Path) -> bool),
    /// The directory name alone identifies it and there is no project to
    /// confirm. Reserved for names no other tool writes.
    Unconditional,
}

impl ProjectMarker {
    fn confirms(&self, root: &Path) -> bool {
        match self {
            Self::AnyFile(names) => names.iter().any(|name| has_marker_file(root, name)),
            Self::AnyExtension(extensions) => extensions
                .iter()
                .any(|extension| has_marker_with_extension(root, extension)),
            Self::Custom(predicate) => predicate(root),
            Self::Unconditional => true,
        }
    }
}

/// A provider written the way an ecosystem describes itself: the file that
/// identifies a project root, and the paths under that root a build can
/// regenerate.
///
/// Artifacts are relative to the root rather than bare directory names, so an
/// artifact that only counts in one place — React Native's `ios/Pods`, sbt's
/// `project/target` — is expressed as the path it actually occupies. A bare
/// `Pods` would claim that directory anywhere it appeared.
pub struct ProjectScanner {
    category: Category,
    artifacts: &'static [&'static str],
    marker: ProjectMarker,
}

impl ProjectScanner {
    pub fn new(
        category: Category,
        artifacts: &'static [&'static str],
        marker: ProjectMarker,
    ) -> Self {
        Self {
            category,
            artifacts,
            marker,
        }
    }
}

impl Scanner for ProjectScanner {
    fn category(&self) -> Category {
        self.category.clone()
    }

    /// The name test stays deliberately loose: it asks only whether some
    /// claimed path ends in this name. `confirm_context` is what decides, and
    /// keeping the two separate preserves the distinction cleanup reports —
    /// a name this provider never claims, versus a name in the wrong place.
    fn matches(&self, name: &str) -> bool {
        self.artifacts
            .iter()
            .any(|artifact| last_segment(artifact) == name)
    }

    fn confirm_context(&self, path: &Path) -> bool {
        self.artifacts.iter().any(|artifact| {
            project_root(path, artifact).is_some_and(|root| self.marker.confirms(root))
        })
    }
}

fn last_segment(artifact: &str) -> &str {
    artifact.rsplit('/').next().unwrap_or(artifact)
}

/// Walks up one level per segment of `artifact`, requiring each to match, and
/// returns the directory the artifact hangs off. `None` when the path does not
/// end in that exact sequence — which is the whole point for the nested ones.
fn project_root<'a>(path: &'a Path, artifact: &str) -> Option<&'a Path> {
    let mut current = path;
    for segment in artifact.rsplit('/') {
        if current.file_name()?.to_str()? != segment {
            return None;
        }
        current = current.parent()?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::{last_segment, project_root, ProjectMarker, ProjectScanner};
    use crate::model::Category;
    use crate::scanner::Scanner;
    use std::path::Path;

    #[test]
    fn a_single_segment_artifact_hangs_off_its_parent() {
        assert_eq!(
            project_root(Path::new("/w/app/node_modules"), "node_modules"),
            Some(Path::new("/w/app"))
        );
    }

    #[test]
    fn a_nested_artifact_requires_every_segment_in_order() {
        assert_eq!(
            project_root(Path::new("/w/app/ios/Pods"), "ios/Pods"),
            Some(Path::new("/w/app"))
        );
        assert_eq!(project_root(Path::new("/w/app/Pods"), "ios/Pods"), None);
        assert_eq!(
            project_root(Path::new("/w/app/android/Pods"), "ios/Pods"),
            None
        );
    }

    #[test]
    fn a_nested_artifact_needs_more_ancestors_than_the_path_has() {
        assert_eq!(project_root(Path::new("/Pods"), "ios/Pods"), None);
        // A single segment does resolve at the filesystem root. Nothing is
        // claimed by it: the marker still has to be found there, and cleanup
        // refuses paths outside the workspace regardless.
        assert_eq!(
            project_root(Path::new("/node_modules"), "node_modules"),
            Some(Path::new("/"))
        );
    }

    #[test]
    fn the_name_test_reads_the_last_segment_of_every_artifact() {
        assert_eq!(last_segment("ios/Pods"), "Pods");
        assert_eq!(last_segment("target"), "target");

        let scanner = ProjectScanner::new(
            Category::ReactNative,
            &[".expo", "ios/Pods"],
            ProjectMarker::Unconditional,
        );
        assert!(scanner.matches(".expo"));
        assert!(scanner.matches("Pods"));
        assert!(!scanner.matches("ios"));
    }

    #[test]
    fn a_nested_artifact_is_not_confirmed_by_its_bare_name() {
        let scanner = ProjectScanner::new(
            Category::ReactNative,
            &["ios/Pods"],
            ProjectMarker::Unconditional,
        );
        // The name matches, so cleanup reaches the context check rather than
        // stopping at the mismatch — and the context check is what refuses.
        assert!(scanner.matches("Pods"));
        assert!(!scanner.confirm_context(Path::new("/w/app/Pods")));
        assert!(scanner.confirm_context(Path::new("/w/app/ios/Pods")));
    }
}
