//! Declared dependencies that nothing in the project refers to.
//!
//! Report-only. Every file is read as text and nothing a project ships is
//! executed, so a configuration that loads a package through code this module
//! cannot see makes it look unused; the confidence level carries that doubt.

mod cargo;
mod node;
mod sources;

use crate::confidence::ConfidenceLevel;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Ecosystem {
    Node,
    Cargo,
}

/// The manifest section a dependency was declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyKind {
    Runtime,
    Development,
    Build,
    Optional,
}

/// Why a dependency with no reference is not reported at full confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnusedReason {
    /// Nothing names it: no import, no config string, no script command.
    NotReferenced,
    /// Tooling loads it by a short name derived from its own
    /// (`eslint-plugin-x` as `x`), which a text search does not resolve.
    ConventionLoaded,
    /// It installs a command that no script runs. It may be run by hand.
    CommandOnly,
    /// A `@types` package, which the compiler can pick up with no reference.
    TypesOnly,
    /// A Cargo entry that sets features or links a native library, which can
    /// be its whole purpose.
    FeaturesOrLinkage,
    /// `node_modules` is absent, so the commands it installs and the packages
    /// that require it as a peer could not be read.
    NotInstalled,
    /// The project has more files than one analysis reads.
    IncompleteScan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnusedDependency {
    pub name: String,
    pub kind: DependencyKind,
    pub confidence: ConfidenceLevel,
    pub reason: UnusedReason,
    /// The package manager's own removal command, run from the manifest's
    /// directory. Shown to be copied; never run by this crate.
    pub remove_with: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyReport {
    pub manifest: PathBuf,
    pub ecosystem: Ecosystem,
    pub declared: usize,
    pub unused: Vec<UnusedDependency>,
}

/// Every manifest analysed inside one project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDependencies {
    pub project: PathBuf,
    pub reports: Vec<DependencyReport>,
    /// Manifests that could not be read or parsed.
    pub unreadable: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("not a package.json or Cargo.toml: {0}")]
    Unsupported(PathBuf),
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse {path}: {message}")]
    Parse { path: PathBuf, message: String },
}

/// Analyses every `package.json` and `Cargo.toml` in `project`, down to
/// `sources::MANIFEST_DEPTH`: monorepo members, Cargo workspace members and a
/// Tauri app's `src-tauri`.
pub fn analyse_project(project: &Path) -> ProjectDependencies {
    let mut reports = Vec::new();
    let mut unreadable = Vec::new();
    for manifest in sources::manifests(project) {
        match analyse_manifest(&manifest) {
            Ok(report) => reports.push(report),
            Err(_) => unreadable.push(manifest),
        }
    }
    ProjectDependencies {
        project: project.to_path_buf(),
        reports,
        unreadable,
    }
}

/// Analyses one `package.json` or `Cargo.toml`. The directory holding it is
/// the scope searched for references, minus nested packages with their own
/// manifest, except for a Node workspace root, which searches its members too.
pub fn analyse_manifest(manifest: &Path) -> Result<DependencyReport, ManifestError> {
    let name = manifest.file_name().and_then(|name| name.to_str());
    let text = std::fs::read_to_string(manifest).map_err(|source| ManifestError::Read {
        path: manifest.to_path_buf(),
        source,
    })?;
    let parse_error = |message: String| ManifestError::Parse {
        path: manifest.to_path_buf(),
        message,
    };

    let (ecosystem, declared, unused) = match name {
        Some("package.json") => {
            let (declared, unused) = node::analyse(manifest, &text).map_err(parse_error)?;
            (Ecosystem::Node, declared, unused)
        }
        Some("Cargo.toml") => {
            let (declared, unused) = cargo::analyse(manifest, &text).map_err(parse_error)?;
            (Ecosystem::Cargo, declared, unused)
        }
        _ => return Err(ManifestError::Unsupported(manifest.to_path_buf())),
    };

    Ok(DependencyReport {
        manifest: manifest.to_path_buf(),
        ecosystem,
        declared,
        unused,
    })
}
