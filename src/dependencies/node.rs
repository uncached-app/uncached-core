use super::sources;
use super::{DependencyKind, UnusedDependency, UnusedReason};
use crate::confidence::ConfidenceLevel;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const SECTIONS: &[(&str, DependencyKind)] = &[
    ("dependencies", DependencyKind::Runtime),
    ("devDependencies", DependencyKind::Development),
    ("optionalDependencies", DependencyKind::Optional),
];

/// `package.json` fields that name packages without using them.
const NAMING_FIELDS: &[&str] = &[
    "name",
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
    "peerDependenciesMeta",
    "bundleDependencies",
    "bundledDependencies",
    "overrides",
    "resolutions",
    "pnpm",
    "workspaces",
];

const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
];

/// Read for quoted package names.
const SOURCE_EXTENSIONS: &[&str] = &[
    "js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts", "vue", "svelte", "astro", "mdx", "json",
    "jsonc", "json5", "html", "css", "scss", "sass", "less", "styl",
];

/// Read for words as well: commands, and YAML's unquoted strings.
const COMMAND_EXTENSIONS: &[&str] = &["yml", "yaml", "toml", "sh", "bash", "zsh", "ps1"];
const COMMAND_FILES: &[&str] = &["Makefile", "Dockerfile", "Procfile", "justfile", "Justfile"];

/// Name prefixes of packages that tooling loads from a short name
/// (`plugins: ["react"]` for `eslint-plugin-react`), which no text search
/// maps back to the package.
const CONVENTION_PREFIXES: &[&str] = &[
    "eslint-plugin-",
    "eslint-config-",
    "prettier-plugin-",
    "babel-plugin-",
    "babel-preset-",
    "@babel/plugin-",
    "@babel/preset-",
    "stylelint-config-",
    "stylelint-plugin-",
    "@commitlint/config-",
    "remark-",
    "rehype-",
];
const SCOPED_CONVENTION_NAMES: &[&str] = &[
    "eslint-plugin",
    "eslint-config",
    "prettier-plugin",
    "prettier-config",
];

/// Packages loaded because their configuration file exists, matched on the
/// file name's start (`postcss.config.mjs`, `.prettierrc.json`).
const LOADED_BY_CONFIG: &[(&str, &[&str])] = &[
    ("postcss", &["postcss.config.", ".postcssrc"]),
    ("tailwindcss", &["tailwind.config."]),
    ("@babel/core", &["babel.config.", ".babelrc"]),
    ("prettier", &["prettier.config.", ".prettierrc"]),
    ("eslint", &["eslint.config.", ".eslintrc"]),
    ("stylelint", &["stylelint.config.", ".stylelintrc"]),
    ("jest", &["jest.config."]),
    ("vitest", &["vitest.config.", "vitest.workspace."]),
    ("lint-staged", &["lint-staged.config.", ".lintstagedrc"]),
];

/// Packages another tool's subcommand loads, matched as two consecutive words.
const LOADED_BY_SUBCOMMAND: &[(&str, &str)] = &[("@astrojs/check", "astro check")];

/// Packages a toolchain loads because a kind of file exists.
const LOADED_BY_FILES: &[(&str, &[&str])] = &[
    ("typescript", &["ts", "tsx", "mts", "cts"]),
    ("sass", &["scss", "sass"]),
    ("sass-embedded", &["scss", "sass"]),
    ("less", &["less"]),
    ("stylus", &["styl"]),
];

#[derive(Default)]
struct References {
    packages: HashSet<String>,
    words: HashSet<String>,
    extensions: HashSet<String>,
    file_names: HashSet<String>,
    complete: bool,
}

struct Installed {
    commands: Vec<String>,
    /// Optional ones included: a host package uses an optional peer when it is
    /// installed, which is the reason to declare one.
    peers: Vec<String>,
}

pub(super) fn analyse(
    manifest: &Path,
    text: &str,
) -> Result<(usize, Vec<UnusedDependency>), String> {
    let package: Map<String, Value> =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    let root = manifest.parent().unwrap_or(Path::new("."));
    let declared = declared(&package);

    let is_workspace_root =
        package.contains_key("workspaces") || root.join("pnpm-workspace.yaml").is_file();
    let references = references(root, !is_workspace_root);
    let installed: Vec<Option<Installed>> = declared
        .iter()
        .map(|(name, _)| installed(root, name))
        .collect();
    let expected_peers: HashSet<&str> = installed
        .iter()
        .flatten()
        .flat_map(|package| package.peers.iter().map(String::as_str))
        .collect();

    let manager = package_manager(root, &package);
    let unused = declared
        .iter()
        .zip(&installed)
        .filter_map(|((name, kind), installed)| {
            let (confidence, reason) =
                assess(name, installed.as_ref(), &references, &expected_peers)?;
            Some(UnusedDependency {
                name: name.clone(),
                kind: *kind,
                confidence,
                reason,
                remove_with: format!("{} {name}", manager.remove_command()),
            })
        })
        .collect();
    Ok((declared.len(), unused))
}

fn declared(package: &Map<String, Value>) -> Vec<(String, DependencyKind)> {
    let mut seen = HashSet::new();
    SECTIONS
        .iter()
        .filter_map(|(field, kind)| Some((package.get(*field)?.as_object()?, *kind)))
        .flat_map(|(section, kind)| section.keys().map(move |name| (name.clone(), kind)))
        .filter(|(name, _)| seen.insert(name.clone()))
        .collect()
}

#[derive(Clone, Copy)]
enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "npm" => Some(Self::Npm),
            "pnpm" => Some(Self::Pnpm),
            "yarn" => Some(Self::Yarn),
            "bun" => Some(Self::Bun),
            _ => None,
        }
    }

    fn remove_command(self) -> &'static str {
        match self {
            Self::Npm => "npm uninstall",
            Self::Pnpm => "pnpm remove",
            Self::Yarn => "yarn remove",
            Self::Bun => "bun remove",
        }
    }
}

const LOCKFILE_MANAGERS: &[(&str, PackageManager)] = &[
    ("pnpm-lock.yaml", PackageManager::Pnpm),
    ("bun.lock", PackageManager::Bun),
    ("bun.lockb", PackageManager::Bun),
    ("yarn.lock", PackageManager::Yarn),
    ("package-lock.json", PackageManager::Npm),
];

/// The manager a `packageManager` field names (`"pnpm@9.1.0"`), else the one
/// whose lockfile sits beside the manifest or above it, which is where a
/// workspace member's lockfile lives. npm when there is neither.
fn package_manager(root: &Path, package: &Map<String, Value>) -> PackageManager {
    let declared = package
        .get("packageManager")
        .and_then(Value::as_str)
        .and_then(|field| PackageManager::from_name(field.split('@').next().unwrap_or(field)));
    declared
        .or_else(|| {
            root.ancestors().find_map(|directory| {
                LOCKFILE_MANAGERS
                    .iter()
                    .find(|(lockfile, _)| directory.join(lockfile).is_file())
                    .map(|(_, manager)| *manager)
            })
        })
        .unwrap_or(PackageManager::Npm)
}

/// `None` when nothing explains the dependency's presence.
fn assess(
    name: &str,
    installed: Option<&Installed>,
    references: &References,
    expected_peers: &HashSet<&str>,
) -> Option<(ConfidenceLevel, UnusedReason)> {
    if references.packages.contains(name)
        || expected_peers.contains(name)
        || loaded_by_files(name, references)
        || loaded_by_config(name, references)
        || loaded_by_subcommand(name, references)
    {
        return None;
    }
    let commands = installed.map_or(&[][..], |package| &package.commands[..]);
    if commands
        .iter()
        .any(|command| references.words.contains(command))
    {
        return None;
    }

    if let Some(typed) = name.strip_prefix("@types/") {
        return (!references.packages.contains(&typed_package(typed)))
            .then_some((ConfidenceLevel::Review, UnusedReason::TypesOnly));
    }
    let medium = |reason| Some((ConfidenceLevel::Medium, reason));
    if !references.complete {
        return medium(UnusedReason::IncompleteScan);
    }
    if is_convention_loaded(name) {
        return medium(UnusedReason::ConventionLoaded);
    }
    if installed.is_none() {
        return medium(UnusedReason::NotInstalled);
    }
    if !commands.is_empty() {
        return medium(UnusedReason::CommandOnly);
    }
    Some((ConfidenceLevel::High, UnusedReason::NotReferenced))
}

/// `@types/scope__name` types `@scope/name`.
fn typed_package(typed: &str) -> String {
    match typed.split_once("__") {
        Some((scope, name)) => format!("@{scope}/{name}"),
        None => typed.to_owned(),
    }
}

fn loaded_by_files(name: &str, references: &References) -> bool {
    LOADED_BY_FILES
        .iter()
        .filter(|(package, _)| *package == name)
        .flat_map(|(_, extensions)| extensions.iter())
        .any(|extension| references.extensions.contains(*extension))
}

fn loaded_by_config(name: &str, references: &References) -> bool {
    LOADED_BY_CONFIG
        .iter()
        .filter(|(package, _)| *package == name)
        .flat_map(|(_, prefixes)| prefixes.iter())
        .any(|prefix| {
            references
                .file_names
                .iter()
                .any(|file| file.starts_with(prefix))
        })
}

fn loaded_by_subcommand(name: &str, references: &References) -> bool {
    LOADED_BY_SUBCOMMAND
        .iter()
        .any(|(package, command)| *package == name && references.words.contains(*command))
}

fn is_convention_loaded(name: &str) -> bool {
    if CONVENTION_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return true;
    }
    let Some((_, unscoped)) = name.strip_prefix('@').and_then(|rest| rest.split_once('/')) else {
        return false;
    };
    SCOPED_CONVENTION_NAMES.iter().any(|convention| {
        unscoped == *convention || unscoped.starts_with(&format!("{convention}-"))
    })
}

fn references(root: &Path, stop_at_nested_packages: bool) -> References {
    let mut references = References::default();
    let boundary = stop_at_nested_packages.then_some("package.json");
    references.complete = sources::walk(root, boundary, is_read, |path, text| {
        record(path, text, &mut references);
    });
    references
}

fn is_read(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if LOCKFILES.contains(&name) {
        return false;
    }
    if COMMAND_FILES.contains(&name)
        || path
            .parent()
            .is_some_and(|parent| parent.ends_with(".husky"))
    {
        return true;
    }
    let extension = extension(path);
    SOURCE_EXTENSIONS.contains(&extension.as_str())
        || COMMAND_EXTENSIONS.contains(&extension.as_str())
        || (name.starts_with('.') && name.ends_with("rc"))
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn record(path: &Path, text: &str, references: &mut References) {
    let extension = extension(path);
    references.extensions.insert(extension.clone());
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        references.file_names.insert(name.to_owned());
    }

    if path.file_name().is_some_and(|name| name == "package.json") {
        record_package_json(text, references);
        return;
    }
    sources::string_literals(text, |literal| {
        add_package(literal, &mut references.packages)
    });
    if !SOURCE_EXTENSIONS.contains(&extension.as_str()) {
        record_words(text, references);
    }
}

/// Each word, and each pair of consecutive words so a subcommand
/// (`astro check`) can be matched.
fn record_words(text: &str, references: &mut References) {
    let mut previous: Option<String> = None;
    sources::words(text, |word| {
        if let Some(previous) = &previous {
            references.words.insert(format!("{previous} {word}"));
        }
        references.words.insert(word.to_owned());
        add_package(word, &mut references.packages);
        previous = Some(word.to_owned());
    });
}

/// Scripts are commands; every other string outside the dependency sections
/// can name a package a tool loads (`"prettier": "@acme/prettier-config"`).
fn record_package_json(text: &str, references: &mut References) {
    let Ok(Value::Object(mut package)) = serde_json::from_str::<Value>(text) else {
        return;
    };
    for field in NAMING_FIELDS {
        package.remove(*field);
    }
    if let Some(Value::Object(scripts)) = package.remove("scripts") {
        for script in scripts.values().filter_map(Value::as_str) {
            record_words(script, references);
        }
    }
    let remaining = Value::Object(package);
    let mut strings = Vec::new();
    collect_strings(&remaining, &mut strings);
    for string in strings {
        add_package(string, &mut references.packages);
    }
}

fn collect_strings<'a>(value: &'a Value, strings: &mut Vec<&'a str>) {
    match value {
        Value::String(string) => strings.push(string),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, strings)),
        Value::Object(fields) => fields
            .values()
            .for_each(|field| collect_strings(field, strings)),
        _ => {}
    }
}

/// Records the package a module specifier resolves to: `react-dom/client` is
/// `react-dom`, `@scope/name/sub` is `@scope/name`. Deno's `npm:name@1` and
/// Sass's `~name/file` spellings are accepted too.
fn add_package(specifier: &str, packages: &mut HashSet<String>) {
    let specifier = specifier.trim();
    let specifier = specifier
        .strip_prefix("npm:")
        .or_else(|| specifier.strip_prefix('~'))
        .unwrap_or(specifier);
    let mut segments = specifier.split('/');
    let first = segments.next().unwrap_or_default();
    let name = if first.starts_with('@') {
        match segments.next() {
            Some(second) => format!("{first}/{}", without_version(second)),
            None => return,
        }
    } else {
        without_version(first).to_owned()
    };
    if !name.is_empty() {
        packages.insert(name);
    }
}

fn without_version(segment: &str) -> &str {
    segment.split('@').next().unwrap_or(segment)
}

/// The installed copy's manifest, found the way Node resolves it: in
/// `node_modules` beside the project or in any ancestor, which covers hoisted
/// monorepo installs.
fn installed(root: &Path, name: &str) -> Option<Installed> {
    let manifest = root
        .ancestors()
        .map(|directory| {
            directory
                .join("node_modules")
                .join(name)
                .join("package.json")
        })
        .find(|candidate| candidate.is_file())?;
    let package: Map<String, Value> =
        serde_json::from_str(&fs::read_to_string(manifest).ok()?).ok()?;
    Some(Installed {
        commands: commands(name, &package),
        peers: peers(&package),
    })
}

fn commands(name: &str, package: &Map<String, Value>) -> Vec<String> {
    match package.get("bin") {
        Some(Value::String(_)) => vec![name.rsplit('/').next().unwrap_or(name).to_owned()],
        Some(Value::Object(commands)) => commands.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

fn peers(package: &Map<String, Value>) -> Vec<String> {
    package
        .get("peerDependencies")
        .and_then(Value::as_object)
        .map(|peers| peers.keys().cloned().collect())
        .unwrap_or_default()
}
