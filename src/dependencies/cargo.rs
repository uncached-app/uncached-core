use super::sources;
use super::{DependencyKind, UnusedDependency, UnusedReason};
use crate::confidence::ConfidenceLevel;
use std::collections::HashSet;
use std::path::Path;
use toml::{Table, Value};

const SECTIONS: &[(&str, DependencyKind)] = &[
    ("dependencies", DependencyKind::Runtime),
    ("dev-dependencies", DependencyKind::Development),
    ("dev_dependencies", DependencyKind::Development),
    ("build-dependencies", DependencyKind::Build),
    ("build_dependencies", DependencyKind::Build),
];

struct Declared<'a> {
    /// The key, which is the name code refers to even when `package` renames
    /// the crate it resolves to.
    key: &'a str,
    kind: DependencyKind,
    /// The section it sits in, which `cargo remove` has to be told.
    section: DependencyKind,
    /// The `cfg(...)` or triple of a `[target.<platform>]` section.
    platform: Option<&'a str>,
    entry: &'a Value,
}

pub(super) fn analyse(
    manifest: &Path,
    text: &str,
) -> Result<(usize, Vec<UnusedDependency>), String> {
    let table: Table = text
        .parse()
        .map_err(|error: toml::de::Error| error.to_string())?;
    // A virtual workspace root declares versions for its members; it has no
    // code of its own to use them.
    if !table.contains_key("package") {
        return Ok((0, Vec::new()));
    }
    let root = manifest.parent().unwrap_or(Path::new("."));
    let declared = declared(&table);
    let ignored = ignored(&table);
    let enabled_by_features = enabled_by_features(&table);

    let mut paths = HashSet::new();
    let complete = sources::walk(
        root,
        Some("Cargo.toml"),
        |path| path.extension().is_some_and(|extension| extension == "rs"),
        |_, text| sources::rust_paths(text, &mut paths),
    );

    let unused = declared
        .iter()
        .filter(|dependency| !ignored.contains(dependency.key))
        .filter(|dependency| !enabled_by_features.contains(dependency.key))
        .filter(|dependency| !paths.contains(&dependency.key.replace('-', "_")))
        .map(|dependency| {
            let (confidence, reason) = assess(dependency, complete);
            UnusedDependency {
                name: dependency.key.to_owned(),
                kind: dependency.kind,
                confidence,
                reason,
                remove_with: remove_with(dependency),
            }
        })
        .collect();
    Ok((declared.len(), unused))
}

fn assess(dependency: &Declared, complete: bool) -> (ConfidenceLevel, UnusedReason) {
    let sets_features = dependency
        .entry
        .get("features")
        .and_then(Value::as_array)
        .is_some_and(|features| !features.is_empty());
    if dependency.key.ends_with("-sys") || sets_features {
        return (ConfidenceLevel::Medium, UnusedReason::FeaturesOrLinkage);
    }
    if !complete {
        return (ConfidenceLevel::Medium, UnusedReason::IncompleteScan);
    }
    (ConfidenceLevel::High, UnusedReason::NotReferenced)
}

/// Every section, including the per-platform ones under `[target.<cfg>]`. A
/// crate declared in several is reported once, as its first declaration.
fn declared(table: &Table) -> Vec<Declared<'_>> {
    let platforms = table
        .get("target")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|targets| {
            targets
                .iter()
                .filter_map(|(platform, scope)| Some((Some(platform.as_str()), scope.as_table()?)))
        });
    let mut seen = HashSet::new();
    std::iter::once((None, table))
        .chain(platforms)
        .flat_map(|(platform, scope)| {
            SECTIONS.iter().filter_map(move |(section, kind)| {
                Some((platform, scope.get(*section)?.as_table()?, *kind))
            })
        })
        .flat_map(|(platform, section, kind)| {
            section.iter().map(move |(key, entry)| Declared {
                key,
                kind: kind_of(kind, entry),
                section: kind,
                platform,
                entry,
            })
        })
        .filter(|dependency| seen.insert(dependency.key))
        .collect()
}

fn remove_with(dependency: &Declared) -> String {
    let mut command = String::from("cargo remove");
    match dependency.section {
        DependencyKind::Development => command.push_str(" --dev"),
        DependencyKind::Build => command.push_str(" --build"),
        DependencyKind::Runtime | DependencyKind::Optional => {}
    }
    if let Some(platform) = dependency.platform {
        command.push_str(&format!(" --target '{platform}'"));
    }
    command.push(' ');
    command.push_str(dependency.key);
    command
}

fn kind_of(section: DependencyKind, entry: &Value) -> DependencyKind {
    let optional = entry
        .get("optional")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if optional {
        DependencyKind::Optional
    } else {
        section
    }
}

/// `[package.metadata.cargo-machete] ignored`, honoured so a project that
/// already told one tool about a false positive is not told about it again.
fn ignored(table: &Table) -> HashSet<&str> {
    table
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("cargo-machete"))
        .and_then(|machete| machete.get("ignored"))
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Dependencies a `[features]` entry names: `dep:x`, `x/feature`, `x?/feature`,
/// and a bare `x`, which is the implicit feature of an optional dependency.
/// Configuring a crate through a feature is a use of it.
fn enabled_by_features(table: &Table) -> HashSet<&str> {
    let Some(features) = table.get("features").and_then(Value::as_table) else {
        return HashSet::new();
    };
    features
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .map(|feature| {
            let feature = feature.strip_prefix("dep:").unwrap_or(feature);
            let name = feature.split_once('/').map_or(feature, |(name, _)| name);
            name.trim_end_matches('?')
        })
        .collect()
}
