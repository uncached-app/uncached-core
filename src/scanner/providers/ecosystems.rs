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
        &["build", ".cxx", ".externalNativeBuild"],
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

/// `Saved` is deliberately absent. Epic's own directory reference puts
/// autosaves, the project's `.ini` configuration and crash logs in it, none of
/// which a build regenerates.
pub fn unreal() -> ProjectScanner {
    ProjectScanner::new(
        Category::UnrealIntermediate,
        &["Binaries", "Build", "DerivedDataCache", "Intermediate"],
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

/// A `composer.json` alone does not make a project: libraries ship one, and
/// anything that bundles a library ships its manifest with it. `vendor` is
/// claimed only where `composer install` can put it back.
pub fn composer() -> ProjectScanner {
    ProjectScanner::new(
        Category::ComposerVendor,
        &["vendor"],
        ProjectMarker::Custom(is_restorable_composer_project),
    )
}

/// True when `root` has a `composer.json`, is not code bundled inside a
/// WordPress extension, and has a `composer.lock` or a repository to restore
/// from.
fn is_restorable_composer_project(root: &Path) -> bool {
    if !has_marker_file(root, "composer.json") || is_bundled_in_wordpress_extension(root) {
        return false;
    }
    has_marker_file(root, "composer.lock") || root.join(".git").exists()
}

/// True strictly below the root of a plugin or theme under `wp-content`; the
/// extension's own root is not bundled.
fn is_bundled_in_wordpress_extension(root: &Path) -> bool {
    let components: Vec<_> = root.components().collect();
    components
        .iter()
        .position(|component| component.as_os_str() == "wp-content")
        .and_then(|index| {
            let kind = components.get(index + 1)?.as_os_str().to_str()?;
            if !matches!(
                kind,
                "plugins" | "themes" | "mu-plugins" | "client-mu-plugins"
            ) {
                return None;
            }
            // `index + 2` is the extension itself.
            Some(components.len() > index + 3)
        })
        .unwrap_or(false)
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

/// `.vercel` itself is never claimed: its `project.json` links the directory
/// to a deployed project, and only `output` beneath it is build output.
pub fn js_build_cache() -> ProjectScanner {
    ProjectScanner::new(
        Category::JsBuildCache,
        &[
            ".parcel-cache",
            ".astro",
            ".docusaurus",
            ".output",
            ".vercel/output",
        ],
        ProjectMarker::AnyFile(&["package.json"]),
    )
}

pub fn serverless_build() -> ProjectScanner {
    ProjectScanner::new(
        Category::ServerlessBuild,
        &[".serverless", ".aws-sam"],
        ProjectMarker::AnyFile(&[
            "serverless.yml",
            "serverless.yaml",
            "serverless.ts",
            "serverless.js",
            "template.yaml",
            "template.yml",
            "samconfig.toml",
        ]),
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
        js_build_cache(),
        serverless_build(),
    ]
}

#[cfg(test)]
mod tests {
    use super::{is_bundled_in_wordpress_extension, is_restorable_composer_project};
    use std::fs;
    use std::path::Path;

    #[test]
    fn only_what_a_wordpress_extension_ships_counts_as_bundled() {
        assert!(is_bundled_in_wordpress_extension(Path::new(
            "/w/site/wp-content/plugins/fluent-smtp/includes/libs/google-api-client/build"
        )));
        assert!(is_bundled_in_wordpress_extension(Path::new(
            "/w/site/wp-content/themes/theme/lib"
        )));
        assert!(!is_bundled_in_wordpress_extension(Path::new(
            "/w/site/wp-content/plugins/my-plugin"
        )));
        assert!(!is_bundled_in_wordpress_extension(Path::new(
            "/w/site/wp-content/uploads/a/b"
        )));
        assert!(!is_bundled_in_wordpress_extension(Path::new(
            "/w/api/src/deep"
        )));
    }

    #[test]
    fn a_composer_root_needs_a_way_to_put_vendor_back() {
        let root =
            std::env::temp_dir().join(format!("uncached-core-composer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        fs::write(root.join("composer.json"), "{}").unwrap();
        assert!(!is_restorable_composer_project(&root));

        fs::write(root.join("composer.lock"), "{}").unwrap();
        assert!(is_restorable_composer_project(&root));

        fs::remove_file(root.join("composer.lock")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        assert!(is_restorable_composer_project(&root));

        let _ = fs::remove_dir_all(&root);
    }
}
