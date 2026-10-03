use std::fs;
use std::path::Path;

/// True when `parent` directly contains a regular file named `name`. Uses
/// `symlink_metadata`, so a symlinked marker file never confirms a deletion
/// context: `is_file` is false for the link itself.
pub fn has_marker_file(parent: &Path, name: &str) -> bool {
    fs::symlink_metadata(parent.join(name)).is_ok_and(|metadata| metadata.is_file())
}

/// True when `parent` directly contains a real directory named `name`.
///
/// Some ecosystems are identified by the directories they require rather than
/// by a file: a Unity project is the `Assets` and `ProjectSettings` pair, and
/// nothing else on disk carries that signature. `symlink_metadata` keeps a
/// symlinked directory from standing in for the real one, for the same reason
/// as `has_marker_file`.
pub fn has_marker_directory(parent: &Path, name: &str) -> bool {
    fs::symlink_metadata(parent.join(name)).is_ok_and(|metadata| metadata.is_dir())
}

/// True when `parent` directly contains any of `names` as a regular file.
pub fn has_any_marker_file(parent: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| has_marker_file(parent, name))
}

/// True when `parent` directly contains an entry whose extension is `extension`.
///
/// Some ecosystems name the project file after the project rather than after
/// the tool: an Xcode project is `Whatever.xcodeproj` and a .NET one is
/// `Whatever.csproj`, so there is no fixed name to look for. Unlike
/// `has_marker_file` this accepts directories, because `.xcodeproj` and
/// `.xcworkspace` are bundles.
///
/// The comparison is case-insensitive: the filesystems these markers live on
/// usually are, and a project saved as `.CSPROJ` is the same project.
pub fn has_marker_with_extension(parent: &Path, extension: &str) -> bool {
    let Ok(entries) = fs::read_dir(parent) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .is_some_and(|found| found.eq_ignore_ascii_case(extension))
    })
}
