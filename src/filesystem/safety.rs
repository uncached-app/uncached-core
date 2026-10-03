use std::path::Path;

pub fn is_protected_directory(name: &str) -> bool {
    name.eq_ignore_ascii_case("$RECYCLE.BIN")
        || name.eq_ignore_ascii_case("System Volume Information")
}

pub fn is_within(root: &Path, target: &Path) -> bool {
    target != root && target.starts_with(root)
}

/// Rejects paths that must never be deleted. Callers pass an already canonical
/// path — the cleanup policy canonicalizes once — so this resolves nothing and
/// performs no filesystem access of its own. Which paths those are is decided
/// by the table in `filesystem::protected`.
pub fn is_safe_to_delete(canonical: &Path) -> bool {
    canonical.parent().is_some() && !super::protected::refuses_deletion(canonical)
}
