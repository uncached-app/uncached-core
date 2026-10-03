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
/// performs no filesystem access of its own.
pub fn is_safe_to_delete(canonical: &Path) -> bool {
    canonical.parent().is_some() && !is_critical_directory(canonical)
}

fn is_critical_directory(path: &Path) -> bool {
    #[cfg(windows)]
    {
        let normalized = path.to_string_lossy().to_lowercase();
        let critical = [
            std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string()),
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string()),
            std::env::var("ProgramFiles(x86)")
                .unwrap_or_else(|_| r"C:\Program Files (x86)".to_string()),
        ];
        critical.iter().any(|value| {
            let value = value.trim_end_matches(['\\', '/']).to_lowercase();
            normalized == value || normalized.starts_with(&(value + "\\"))
        })
    }
    #[cfg(not(windows))]
    {
        matches!(
            path.to_str(),
            Some("/usr" | "/bin" | "/sbin" | "/etc" | "/var" | "/home" | "/root" | "/opt")
        )
    }
}
