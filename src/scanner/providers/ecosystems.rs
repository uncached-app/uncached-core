//! Project types whose artifacts are described by a marker and a path list.
//!
//! The definitions follow Kondo (`kondo-lib`), which has years of real use
//! behind its choices. Two of them are load-bearing omissions rather than
//! oversights, and they are kept here on purpose:
//!
//! - No Python virtual environment is ever claimed. `venv`, `env` and their
//!   variants are ordinary directory names, and rebuilding one costs network.
//! - Fetched dependencies are claimed only where the ecosystem treats them as
//!   build output. Elixir's `deps/` is not claimed for that reason.

use super::project::{ProjectMarker, ProjectScanner};
use crate::filesystem::{has_marker_directory, has_marker_file};
use crate::model::Category;
use std::path::Path;

pub fn maven() -> ProjectScanner {
    ProjectScanner::new(
        Category::MavenTarget,
        &["target"],
        ProjectMarker::AnyFile(&["pom.xml"]),
    )
}

pub fn sbt() -> ProjectScanner {
    ProjectScanner::new(
        Category::SbtTarget,
        &["target", "project/target"],
        ProjectMarker::AnyFile(&["build.sbt"]),
    )
}

/// Gradle's per-project output. The `.gradle` cache beside it is tool state and
/// belongs to its own provider, which reports it wherever it is found.
pub fn gradle_build() -> ProjectScanner {
    ProjectScanner::new(
        Category::GradleBuild,
        &["build"],
        ProjectMarker::AnyFile(&[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ]),
    )
}

pub fn cmake() -> ProjectScanner {
    ProjectScanner::new(
        Category::CMakeBuild,
        &["build", "cmake-build-debug", "cmake-build-release"],
        ProjectMarker::AnyFile(&["CMakeLists.txt"]),
    )
}

/// Unity is recognized by the project file Unity itself generates. `Library`
/// and `Temp` are names the operating system also uses, so nothing here may be
/// claimed without that file directly beside it.
pub fn unity() -> ProjectScanner {
    ProjectScanner::new(
        Category::UnityLibrary,
        &[
            "Library",
            "Temp",
            "Obj",
            "Logs",
            "MemoryCaptures",
            "Build",
            "Builds",
        ],
        ProjectMarker::AnyFile(&["Assembly-CSharp.csproj"]),
    )
}

pub fn unreal() -> ProjectScanner {
    ProjectScanner::new(
        Category::UnrealIntermediate,
        &[
            "Binaries",
            "Build",
            "Saved",
            "DerivedDataCache",
            "Intermediate",
        ],
        ProjectMarker::AnyExtension(&["uproject"]),
    )
}

pub fn godot() -> ProjectScanner {
    ProjectScanner::new(
        Category::GodotCache,
        &[".godot"],
        ProjectMarker::AnyFile(&["project.godot"]),
    )
}

pub fn elixir() -> ProjectScanner {
    ProjectScanner::new(
        Category::ElixirBuild,
        &["_build", ".elixir-tools", ".elixir_ls", ".lexical"],
        ProjectMarker::AnyFile(&["mix.exs"]),
    )
}

pub fn haskell() -> ProjectScanner {
    ProjectScanner::new(
        Category::HaskellBuild,
        &[".stack-work", "dist-newstyle"],
        ProjectMarker::AnyFile(&["stack.yaml", "cabal.project"]),
    )
}

pub fn zig() -> ProjectScanner {
    ProjectScanner::new(
        Category::ZigCache,
        &["zig-cache", ".zig-cache", "zig-out"],
        ProjectMarker::AnyFile(&["build.zig"]),
    )
}

/// Flutter's output. `.dart_tool` is claimed by its own provider, which covers
/// plain Dart packages as well.
pub fn flutter() -> ProjectScanner {
    ProjectScanner::new(
        Category::FlutterBuild,
        &[
            "build",
            "linux/flutter/ephemeral",
            "windows/flutter/ephemeral",
        ],
        ProjectMarker::AnyFile(&["pubspec.yaml"]),
    )
}

pub fn jupyter() -> ProjectScanner {
    ProjectScanner::new(
        Category::JupyterCheckpoints,
        &[".ipynb_checkpoints"],
        ProjectMarker::AnyExtension(&["ipynb"]),
    )
}

pub fn pixi() -> ProjectScanner {
    ProjectScanner::new(
        Category::PixiEnv,
        &[".pixi"],
        ProjectMarker::AnyFile(&["pixi.toml"]),
    )
}

pub fn composer() -> ProjectScanner {
    ProjectScanner::new(
        Category::ComposerVendor,
        &["vendor"],
        ProjectMarker::AnyFile(&["composer.json"]),
    )
}

pub fn terraform() -> ProjectScanner {
    ProjectScanner::new(
        Category::TerraformDir,
        &[".terraform"],
        ProjectMarker::AnyFile(&[".terraform.lock.hcl"]),
    )
}

pub fn cocoapods() -> ProjectScanner {
    ProjectScanner::new(
        Category::CocoapodsPods,
        &["Pods"],
        ProjectMarker::AnyFile(&["Podfile"]),
    )
}

/// React Native keeps build output inside the platform directories, where the
/// same names would otherwise be claimed by whatever tool owns that platform.
/// Registered ahead of Gradle and CocoaPods so an RN app reports as one project
/// rather than as three unrelated ones.
pub fn react_native() -> ProjectScanner {
    ProjectScanner::new(
        Category::ReactNative,
        &[
            ".expo",
            ".metro",
            "android/build",
            "android/.gradle",
            "ios/build",
            "ios/DerivedData",
            "ios/Pods",
        ],
        ProjectMarker::Custom(is_react_native),
    )
}

fn is_react_native(root: &Path) -> bool {
    has_marker_file(root, "package.json")
        && (has_marker_directory(root, "ios") || has_marker_directory(root, "android"))
}

pub fn nuxt() -> ProjectScanner {
    ProjectScanner::new(
        Category::NuxtCache,
        &[".nuxt"],
        ProjectMarker::AnyFile(&["package.json"]),
    )
}

pub fn sveltekit() -> ProjectScanner {
    ProjectScanner::new(
        Category::SvelteKitCache,
        &[".svelte-kit"],
        ProjectMarker::AnyFile(&["package.json"]),
    )
}

/// Every project-type provider defined here, in the order a set should register
/// them: the ones that claim a nested path come before the ones that would
/// claim the same directory by its bare name.
pub fn all() -> Vec<ProjectScanner> {
    vec![
        react_native(),
        maven(),
        sbt(),
        gradle_build(),
        cmake(),
        unity(),
        unreal(),
        godot(),
        elixir(),
        haskell(),
        zig(),
        flutter(),
        jupyter(),
        pixi(),
        composer(),
        terraform(),
        cocoapods(),
        nuxt(),
        sveltekit(),
    ]
}
