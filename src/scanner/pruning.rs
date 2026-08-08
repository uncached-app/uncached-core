use crate::filesystem::is_protected_directory;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const PRUNED_PATH_INDEX_THRESHOLD: usize = 8;

#[derive(Default)]
#[cfg(feature = "bench")]
pub struct PrunedPaths {
    paths: Vec<PathBuf>,
    index: Option<HashSet<PathBuf>>,
}

#[derive(Default)]
#[cfg(not(feature = "bench"))]
pub(crate) struct PrunedPaths {
    paths: Vec<PathBuf>,
    index: Option<HashSet<PathBuf>>,
}

impl PrunedPaths {
    pub fn insert(&mut self, path: PathBuf) {
        if let Some(index) = &mut self.index {
            index.insert(path);
            return;
        }
        if self.paths.iter().any(|existing| existing == &path) {
            return;
        }
        self.paths.push(path);
        if self.paths.len() > PRUNED_PATH_INDEX_THRESHOLD {
            self.index = Some(self.paths.iter().cloned().collect());
        }
    }

    pub fn contains_ancestor(&self, path: &Path) -> bool {
        match &self.index {
            Some(index) => path.ancestors().any(|ancestor| index.contains(ancestor)),
            None => self.paths.iter().any(|pruned| path.starts_with(pruned)),
        }
    }

    #[cfg(test)]
    pub(crate) fn uses_index(&self) -> bool {
        self.index.is_some()
    }
}

pub(crate) fn should_prune_directory(name: &str) -> bool {
    is_protected_directory(name)
        || matches!(name, ".git" | ".hg" | ".svn" | ".bzr" | ".idea" | ".vscode")
}

#[cfg(test)]
mod tests {
    use super::{PrunedPaths, PRUNED_PATH_INDEX_THRESHOLD};
    use std::path::{Path, PathBuf};

    #[test]
    fn pruned_lookup_respects_path_components() {
        let mut pruned = PrunedPaths::default();
        pruned.insert(PathBuf::from("/repo/node_modules"));
        assert!(pruned.contains_ancestor(Path::new("/repo/node_modules")));
        assert!(pruned.contains_ancestor(Path::new("/repo/node_modules/pkg/file")));
        assert!(!pruned.contains_ancestor(Path::new("/repo/node_modules-cache/file")));
        assert!(!pruned.contains_ancestor(Path::new("/other/node_modules/file")));
    }

    #[test]
    fn pruned_lookup_handles_empty_and_nested_paths() {
        let mut pruned = PrunedPaths::default();
        assert!(!pruned.contains_ancestor(Path::new("/repo/node_modules/file")));
        pruned.insert(PathBuf::from("/repo"));
        pruned.insert(PathBuf::from("/repo/node_modules"));
        assert!(pruned.contains_ancestor(Path::new("/repo/node_modules/file")));
    }

    #[test]
    fn pruned_lookup_uses_linear_mode_for_small_sets() {
        let mut pruned = PrunedPaths::default();
        for target in 0..PRUNED_PATH_INDEX_THRESHOLD {
            pruned.insert(PathBuf::from(format!("/repo/target-{target}")));
        }
        assert!(!pruned.uses_index());
        assert!(pruned.contains_ancestor(Path::new("/repo/target-0/file")));
    }

    #[test]
    fn pruned_lookup_switches_to_indexed_mode_after_threshold() {
        let mut pruned = PrunedPaths::default();
        for target in 0..=PRUNED_PATH_INDEX_THRESHOLD {
            pruned.insert(PathBuf::from(format!("/repo/target-{target}")));
        }
        assert!(pruned.uses_index());
        assert!(pruned.contains_ancestor(Path::new("/repo/target-0/file")));
        assert!(!pruned.contains_ancestor(Path::new("/repo/target-0-cache/file")));
    }
}
