use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Files read per analysis. Past it the reference set is incomplete, and the
/// caller lowers its confidence rather than trusting a partial search.
pub(super) const FILE_LIMIT: usize = 20_000;
/// Larger files are bundles, generated code or data, not hand-written source.
const FILE_SIZE_LIMIT: u64 = 512 * 1024;
const LITERAL_LENGTH_LIMIT: usize = 256;
/// Directory levels below a project searched for manifests.
pub(super) const MANIFEST_DEPTH: usize = 3;
const MANIFEST_LIMIT: usize = 64;
const MANIFESTS: &[&str] = &["package.json", "Cargo.toml"];

/// Build output, installed packages and caches: their references belong to
/// other packages and would make every dependency look used.
const SKIPPED_DIRECTORIES: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    "vendor",
    "Pods",
    "DerivedData",
    "__pycache__",
];

/// Hidden directories that hold commands and tool configuration. Every other
/// hidden directory (`.git`, `.next`, `.astro`, `.expo`, `.turbo`...) is
/// skipped.
const READ_HIDDEN_DIRECTORIES: &[&str] = &[".github", ".husky", ".circleci", ".storybook"];

/// Visits every regular file under `root` that `accept` takes, with its text.
/// A subdirectory holding its own `nested_boundary` file is another package
/// and is not entered. Symlinks are never followed. Returns `false` when
/// `FILE_LIMIT` stopped the walk early.
pub(super) fn walk(
    root: &Path,
    nested_boundary: Option<&str>,
    accept: impl Fn(&Path) -> bool,
    mut visit: impl FnMut(&Path, &str),
) -> bool {
    let mut pending = vec![root.to_path_buf()];
    let mut read = 0usize;

    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                if enters(&path, nested_boundary) {
                    pending.push(path);
                }
                continue;
            }
            if !file_type.is_file() || !accept(&path) || !small_enough(&entry) {
                continue;
            }
            if read == FILE_LIMIT {
                return false;
            }
            read += 1;
            if let Ok(text) = fs::read_to_string(&path) {
                visit(&path, &text);
            }
        }
    }
    true
}

/// The manifests in `project` and in its subdirectories down to
/// `MANIFEST_DEPTH`, sorted, at most `MANIFEST_LIMIT` of them.
pub(super) fn manifests(project: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![(project.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        found.extend(
            MANIFESTS
                .iter()
                .map(|name| directory.join(name))
                .filter(|manifest| manifest.is_file()),
        );
        if depth == MANIFEST_DEPTH {
            continue;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let is_directory = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
            if is_directory && enters(&entry.path(), None) {
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    found.sort();
    found.truncate(MANIFEST_LIMIT);
    found
}

fn enters(directory: &Path, nested_boundary: Option<&str>) -> bool {
    let Some(name) = directory.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if SKIPPED_DIRECTORIES.contains(&name) {
        return false;
    }
    if name.starts_with('.') && !READ_HIDDEN_DIRECTORIES.contains(&name) {
        return false;
    }
    nested_boundary.is_none_or(|marker| !directory.join(marker).is_file())
}

fn small_enough(entry: &fs::DirEntry) -> bool {
    entry
        .metadata()
        .is_ok_and(|metadata| metadata.len() <= FILE_SIZE_LIMIT)
}

/// Every quoted string in `text`: `'…'` and `"…"` on one line, `` `…` `` across
/// lines. Comments are not told apart from code, which can only add references.
pub(super) fn string_literals(text: &str, mut found: impl FnMut(&str)) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let quote = bytes[index];
        if !matches!(quote, b'"' | b'\'' | b'`') {
            index += 1;
            continue;
        }
        let start = index + 1;
        match closing_quote(bytes, start, quote) {
            Some(end) => {
                if end > start {
                    // Quotes are ASCII, so both ends sit on char boundaries.
                    found(&text[start..end]);
                }
                index = end + 1;
            }
            None => index = start,
        }
    }
}

fn closing_quote(bytes: &[u8], start: usize, quote: u8) -> Option<usize> {
    let mut end = start;
    while end < bytes.len() && end - start <= LITERAL_LENGTH_LIMIT {
        match bytes[end] {
            b'\\' => end += 2,
            b'\n' if quote != b'`' => return None,
            byte if byte == quote => return Some(end),
            _ => end += 1,
        }
    }
    None
}

/// Whitespace- and punctuation-separated words, for commands in scripts, CI
/// files and unquoted YAML. A path-like word also yields its last segment, so
/// `node_modules/.bin/tsc` names `tsc`.
pub(super) fn words(text: &str, mut found: impl FnMut(&str)) {
    let is_word = |c: char| c.is_ascii_alphanumeric() || "@/._:+-".contains(c);
    for word in text.split(|c: char| !is_word(c)) {
        let word = word.trim_matches(|c: char| ".:".contains(c));
        if word.is_empty() {
            continue;
        }
        found(word);
        if let Some((_, last)) = word.rsplit_once('/') {
            if !last.is_empty() {
                found(last);
            }
        }
    }
}

/// Crate names a Rust file refers to: any identifier followed by `::`, and
/// every identifier inside a `use` or `extern crate` item.
pub(super) fn rust_paths(text: &str, paths: &mut HashSet<String>) {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_use_item = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b';' {
            in_use_item = false;
        }
        if !(byte.is_ascii_alphabetic() || byte == b'_') {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        let identifier = &text[start..index];
        if identifier == "use" || identifier == "extern" {
            in_use_item = true;
        } else if in_use_item || text[index..].trim_start().starts_with("::") {
            paths.insert(identifier.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literals(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        string_literals(text, |literal| found.push(literal.to_owned()));
        found
    }

    #[test]
    fn literals_cover_the_three_quotes_and_skip_escapes() {
        let text = r#"import a from "react"; require('x/y'); const t = `@s/p`; "a\"b""#;
        assert_eq!(literals(text), ["react", "x/y", "@s/p", r#"a\"b"#]);
    }

    #[test]
    fn an_apostrophe_in_prose_does_not_swallow_the_next_line() {
        assert_eq!(literals("// don't\nimport 'zod'"), ["zod"]);
    }

    #[test]
    fn words_name_the_command_at_the_end_of_a_path() {
        let mut found = Vec::new();
        words("node_modules/.bin/tsc --noEmit && astro check", |word| {
            found.push(word.to_owned())
        });
        assert!(found.contains(&"tsc".to_owned()));
        assert!(found.contains(&"astro".to_owned()));
        assert!(found.contains(&"--noEmit".to_owned()));
    }

    #[test]
    fn rust_paths_come_from_paths_and_use_items() {
        let mut paths = HashSet::new();
        let text = "use serde::{Deserialize};\nextern crate libc;\nuse {rayon, jwalk as walk};\n\
                    fn f() { tokio :: spawn(x); let trash = 1; }";
        rust_paths(text, &mut paths);
        for name in ["serde", "libc", "rayon", "jwalk", "tokio"] {
            assert!(paths.contains(name), "{name} missing from {paths:?}");
        }
        assert!(!paths.contains("trash"));
    }
}
