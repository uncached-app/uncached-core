use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use uncached_core::{traverse_workspace, Category, IgnoreRules, ProviderSet, TraversalControl};

static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "uncached-core-providers-{}-{}",
        std::process::id(),
        FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn categories(root: &Path, providers: &ProviderSet) -> Vec<Category> {
    let cancelled = AtomicBool::new(false);
    let mut found = Vec::new();
    traverse_workspace(root, providers, &cancelled, |entry| {
        if let Some(category) = entry.category.clone() {
            found.push(category);
            TraversalControl::Prune
        } else {
            TraversalControl::Continue
        }
    })
    .unwrap();
    found
}

#[test]
fn all_registers_every_scanner_in_deterministic_order() {
    let providers = ProviderSet::all();

    let expected = [
        // React Native leads: it claims paths inside `ios/` and `android/`
        // that the platform's own tooling would otherwise claim by bare name.
        Category::ReactNative,
        Category::NodeModules,
        Category::CargoTarget,
        Category::NextJs,
        Category::Turbo,
        Category::PyCache,
        Category::Gradle,
        Category::XcodeDerivedData,
        Category::SwiftBuild,
        Category::DotNetObj,
        Category::DartTool,
        Category::MavenTarget,
        Category::SbtTarget,
        Category::GradleBuild,
        Category::CMakeBuild,
        Category::UnityLibrary,
        Category::UnrealIntermediate,
        Category::GodotCache,
        Category::ElixirBuild,
        Category::HaskellBuild,
        Category::ZigCache,
        Category::FlutterBuild,
        Category::JupyterCheckpoints,
        Category::PixiEnv,
        Category::ComposerVendor,
        Category::TerraformDir,
        Category::CocoapodsPods,
        Category::NuxtCache,
        Category::SvelteKitCache,
    ];
    assert_eq!(providers.len(), expected.len());
    assert_eq!(providers.categories().collect::<Vec<_>>(), expected);
    assert_eq!(ProviderSet::free().len(), 1);
    assert!(ProviderSet::free()
        .scanner_for(&Category::NodeModules)
        .is_some());
}

#[test]
fn all_scanners_require_expected_context_and_reject_bare_cargo_target() {
    let root = fixture();

    fs::create_dir_all(root.join("cargo/target")).unwrap();
    fs::create_dir_all(root.join("project/target")).unwrap();
    fs::write(root.join("project/Cargo.toml"), b"[package]").unwrap();

    fs::create_dir_all(root.join("next/.next")).unwrap();
    fs::write(root.join("next/package.json"), b"{}").unwrap();
    fs::create_dir_all(root.join("bare-next/.next")).unwrap();

    fs::create_dir_all(root.join("turbo-package/.turbo")).unwrap();
    fs::write(root.join("turbo-package/package.json"), b"{}").unwrap();
    fs::create_dir_all(root.join("turbo-config/.turbo")).unwrap();
    fs::write(root.join("turbo-config/turbo.json"), b"{}").unwrap();
    fs::create_dir_all(root.join("bare-turbo/.turbo")).unwrap();

    fs::create_dir_all(root.join("python/__pycache__")).unwrap();
    fs::create_dir_all(root.join("gradle/.gradle")).unwrap();

    let found = categories(&root, &ProviderSet::all());
    assert_eq!(found.len(), 6);
    assert_eq!(
        found
            .iter()
            .filter(|category| **category == Category::CargoTarget)
            .count(),
        1
    );
    assert_eq!(
        found
            .iter()
            .filter(|category| **category == Category::NextJs)
            .count(),
        1
    );
    assert_eq!(
        found
            .iter()
            .filter(|category| **category == Category::Turbo)
            .count(),
        2
    );
    assert_eq!(
        found
            .iter()
            .filter(|category| **category == Category::PyCache)
            .count(),
        1
    );
    assert_eq!(
        found
            .iter()
            .filter(|category| **category == Category::Gradle)
            .count(),
        1
    );

    let _ = fs::remove_dir_all(root);
}

/// `bin`, `obj` and `build` are the most reused directory names in this whole
/// catalogue, so their providers are the ones where a missing context check
/// would be most destructive. Each valid case here has an invalid twin.
#[test]
fn ecosystem_providers_match_only_beside_their_project_file() {
    let root = fixture();

    fs::create_dir_all(root.join("xcode-valid/DerivedData")).unwrap();
    fs::create_dir_all(root.join("xcode-valid/App.xcodeproj")).unwrap();
    fs::create_dir_all(root.join("xcode-workspace/build")).unwrap();
    fs::create_dir_all(root.join("xcode-workspace/App.xcworkspace")).unwrap();
    fs::create_dir_all(root.join("plain-build/build")).unwrap();
    fs::create_dir_all(root.join("plain-derived/DerivedData")).unwrap();

    fs::create_dir_all(root.join("swift-valid/.build")).unwrap();
    fs::write(root.join("swift-valid/Package.swift"), b"// swift-tools").unwrap();
    fs::create_dir_all(root.join("swift-invalid/.build")).unwrap();

    fs::create_dir_all(root.join("dotnet-valid/obj")).unwrap();
    fs::create_dir_all(root.join("dotnet-valid/bin")).unwrap();
    fs::write(root.join("dotnet-valid/App.csproj"), b"<Project />").unwrap();
    fs::create_dir_all(root.join("dotnet-invalid/obj")).unwrap();
    fs::create_dir_all(root.join("dotnet-invalid/bin")).unwrap();

    fs::create_dir_all(root.join("flutter-valid/.dart_tool")).unwrap();
    fs::write(root.join("flutter-valid/pubspec.yaml"), b"name: app").unwrap();
    fs::create_dir_all(root.join("flutter-invalid/.dart_tool")).unwrap();

    let mut found = Vec::new();
    let cancelled = AtomicBool::new(false);
    traverse_workspace(
        &root,
        &ProviderSet::all(),
        &cancelled,
        |entry| match entry.category.clone() {
            Some(category) => {
                found.push((entry.path.clone(), category));
                TraversalControl::Prune
            }
            None => TraversalControl::Continue,
        },
    )
    .unwrap();

    found.sort_by(|left, right| left.0.cmp(&right.0));
    let expected = [
        ("dotnet-valid/bin", Category::DotNetObj),
        ("dotnet-valid/obj", Category::DotNetObj),
        ("flutter-valid/.dart_tool", Category::DartTool),
        ("swift-valid/.build", Category::SwiftBuild),
        ("xcode-valid/DerivedData", Category::XcodeDerivedData),
        ("xcode-workspace/build", Category::XcodeDerivedData),
    ]
    .into_iter()
    .map(|(path, category)| (uncached_core::canonical(root.join(path)).unwrap(), category))
    .collect::<Vec<_>>();

    assert_eq!(found, expected);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn contextual_providers_reject_same_basename_decoy_contexts() {
    let root = fixture();

    fs::create_dir_all(root.join("node-valid/node_modules")).unwrap();
    fs::write(root.join("node-valid/package.json"), b"{}").unwrap();
    fs::create_dir_all(root.join("node-invalid/node_modules")).unwrap();

    fs::create_dir_all(root.join("cargo-valid/target")).unwrap();
    fs::write(root.join("cargo-valid/Cargo.toml"), b"[package]").unwrap();
    fs::create_dir_all(root.join("cargo-invalid/target")).unwrap();

    fs::create_dir_all(root.join("next-valid/.next")).unwrap();
    fs::write(root.join("next-valid/package.json"), b"{}").unwrap();
    fs::create_dir_all(root.join("next-invalid/.next")).unwrap();

    fs::create_dir_all(root.join("turbo-valid/.turbo")).unwrap();
    fs::write(root.join("turbo-valid/package.json"), b"{}").unwrap();
    fs::create_dir_all(root.join("turbo-invalid/.turbo")).unwrap();

    let mut found = Vec::new();
    let cancelled = AtomicBool::new(false);
    traverse_workspace(&root, &ProviderSet::all(), &cancelled, |entry| {
        if entry.category.is_some() {
            found.push((entry.path.clone(), entry.category.clone().unwrap()));
            TraversalControl::Prune
        } else {
            TraversalControl::Continue
        }
    })
    .unwrap();

    found.sort_by(|left, right| left.0.cmp(&right.0));
    let expected = [
        ("cargo-valid/target", Category::CargoTarget),
        ("next-valid/.next", Category::NextJs),
        ("node-valid/node_modules", Category::NodeModules),
        ("turbo-valid/.turbo", Category::Turbo),
    ]
    .into_iter()
    .map(|(path, category)| (uncached_core::canonical(root.join(path)).unwrap(), category))
    .collect::<Vec<_>>();

    assert_eq!(found, expected);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn context_free_providers_accept_reachable_same_basename_directories() {
    let root = fixture();

    fs::create_dir_all(root.join("python-valid/__pycache__")).unwrap();
    fs::create_dir_all(root.join("python-invalid/__pycache__")).unwrap();
    fs::create_dir_all(root.join("gradle-valid/.gradle")).unwrap();
    fs::create_dir_all(root.join("gradle-invalid/.gradle")).unwrap();

    let found = categories(&root, &ProviderSet::all());
    assert_eq!(
        found
            .iter()
            .filter(|category| **category == Category::PyCache)
            .count(),
        2
    );
    assert_eq!(
        found
            .iter()
            .filter(|category| **category == Category::Gradle)
            .count(),
        2
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn contextual_policy_excludes_name_only_cache_providers() {
    let root = fixture();

    fs::create_dir_all(root.join("python/__pycache__")).unwrap();
    fs::create_dir_all(root.join("gradle/.gradle")).unwrap();
    fs::create_dir_all(root.join("node/node_modules")).unwrap();
    fs::write(root.join("node/package.json"), b"{}").unwrap();

    let found = categories(&root, &ProviderSet::all_contextual());

    assert_eq!(found, vec![Category::NodeModules]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn git_subtrees_are_pruned_before_provider_matching() {
    let root = fixture();
    fs::create_dir_all(root.join(".git/fake-python/__pycache__")).unwrap();
    fs::create_dir_all(root.join(".git/fake-gradle/.gradle")).unwrap();

    let mut visited = Vec::new();
    let cancelled = AtomicBool::new(false);
    traverse_workspace(&root, &ProviderSet::all(), &cancelled, |entry| {
        visited.push(entry.path.clone());
        TraversalControl::Continue
    })
    .unwrap();

    assert!(!visited
        .iter()
        .any(|path| path.starts_with(root.join(".git"))));
    let _ = fs::remove_dir_all(root);
}

/// The user's rules are the one input that overrules everything else the crate
/// decides, so they are checked where a deletion is authorized as well as where
/// a directory is found. These two tests are the pair: the walk never reaches
/// the path, and cleanup refuses it even when handed it directly.
#[test]
fn an_ignored_path_is_never_walked_into() {
    let root = fixture();
    fs::create_dir_all(root.join("keep/node_modules")).unwrap();
    fs::write(root.join("keep/package.json"), b"{}").unwrap();
    fs::create_dir_all(root.join("skip/node_modules")).unwrap();
    fs::write(root.join("skip/package.json"), b"{}").unwrap();

    let canonical = uncached_core::canonical(&root).unwrap();
    let (rules, errors) =
        IgnoreRules::compile(&format!("{}/skip", canonical.to_string_lossy()), None);
    assert!(errors.is_empty(), "{errors:?}");
    let providers = ProviderSet::all().with_ignore_rules(rules);

    let mut visited = Vec::new();
    let cancelled = AtomicBool::new(false);
    traverse_workspace(&root, &providers, &cancelled, |entry| {
        visited.push(entry.path.clone());
        TraversalControl::Continue
    })
    .unwrap();

    assert!(visited
        .iter()
        .any(|path| path.ends_with("keep/node_modules")));
    assert!(
        !visited
            .iter()
            .any(|path| path.starts_with(canonical.join("skip"))),
        "an excluded directory must not be searched either"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cleanup_refuses_a_target_a_rule_excludes() {
    let root = fixture();
    let target_path = root.join("project/node_modules");
    fs::create_dir_all(&target_path).unwrap();
    fs::write(root.join("project/package.json"), b"{}").unwrap();

    let canonical_root = uncached_core::canonical(&root).unwrap();
    let canonical_target = uncached_core::canonical(&target_path).unwrap();
    let target = uncached_core::ScanTarget::new(canonical_target.clone(), Category::NodeModules, 0)
        .with_identity(uncached_core::filesystem::file_identity(&canonical_target));

    // Without a rule the same target validates, so the refusal below is the
    // rule doing it and not some unrelated part of the policy.
    assert!(
        uncached_core::validate_clean_target(&canonical_root, &ProviderSet::all(), &target,)
            .is_ok()
    );

    let (rules, errors) = IgnoreRules::compile(&canonical_target.to_string_lossy(), None);
    assert!(errors.is_empty(), "{errors:?}");
    let error = uncached_core::validate_clean_target(
        &canonical_root,
        &ProviderSet::all().with_ignore_rules(rules),
        &target,
    )
    .expect_err("an excluded target must not be cleanable");

    assert!(
        matches!(error, uncached_core::CoreError::TargetIgnoredByRule { .. }),
        "unexpected error: {error:?}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Unreal's `Saved` is not an artifact, whatever its siblings are.
///
/// Epic's directory reference puts autosaves, the project's `.ini` files and
/// crash logs in it. A cleaner that offers it beside `Intermediate` is offering
/// to delete work, so the provider claims the four that a build regenerates and
/// leaves that one alone.
#[test]
fn an_unreal_project_keeps_what_a_build_does_not_regenerate() {
    let root = fixture();
    let project = root.join("game");
    for name in [
        "Binaries",
        "Build",
        "DerivedDataCache",
        "Intermediate",
        "Saved",
    ] {
        fs::create_dir_all(project.join(name)).unwrap();
    }
    fs::write(project.join("game.uproject"), b"{}").unwrap();

    let found = categories(&root, &ProviderSet::all());

    assert_eq!(
        found
            .iter()
            .filter(|category| **category == Category::UnrealIntermediate)
            .count(),
        4,
        "Saved holds autosaves, configuration and crash logs"
    );
}

/// The names these providers claim are also ordinary directory names, and one
/// of them is `Library`. Each has to be refused without its ecosystem's marker
/// directly beside it, or the provider is a liability rather than a feature.
#[test]
fn a_dangerous_name_is_refused_without_its_project_marker() {
    let root = fixture();

    // Unity's names, with and without the project file Unity generates.
    fs::create_dir_all(root.join("home/Library")).unwrap();
    fs::create_dir_all(root.join("game/Library")).unwrap();
    fs::write(root.join("game/Assembly-CSharp.csproj"), b"<Project/>").unwrap();

    // `vendor` is Composer's only with a composer.json beside it.
    fs::create_dir_all(root.join("notes/vendor")).unwrap();
    fs::create_dir_all(root.join("php/vendor")).unwrap();
    fs::write(root.join("php/composer.json"), b"{}").unwrap();

    // `build` belongs to whichever ecosystem's marker is present.
    fs::create_dir_all(root.join("plain/build")).unwrap();
    fs::create_dir_all(root.join("cmake/build")).unwrap();
    fs::write(root.join("cmake/CMakeLists.txt"), b"project(x)").unwrap();

    let found = categories(&root, &ProviderSet::all());

    assert_eq!(
        found
            .iter()
            .filter(|c| **c == Category::UnityLibrary)
            .count(),
        1,
        "only the directory beside Assembly-CSharp.csproj is Unity's"
    );
    assert_eq!(
        found
            .iter()
            .filter(|c| **c == Category::ComposerVendor)
            .count(),
        1
    );
    assert_eq!(
        found.iter().filter(|c| **c == Category::CMakeBuild).count(),
        1
    );
}

/// Kondo does not remove virtual environments, and neither do we: the names are
/// ordinary words and rebuilding one costs network.
#[test]
fn no_provider_claims_a_python_virtual_environment() {
    let root = fixture();

    for name in ["venv", ".venv", "env", "virtualenv"] {
        fs::create_dir_all(root.join(format!("project/{name}/lib"))).unwrap();
        fs::write(
            root.join(format!("project/{name}/pyvenv.cfg")),
            b"home = /usr",
        )
        .unwrap();
    }
    fs::write(root.join("project/pyproject.toml"), b"[project]").unwrap();
    fs::create_dir_all(root.join("project/build")).unwrap();
    fs::create_dir_all(root.join("project/dist")).unwrap();
    fs::create_dir_all(root.join("project/__pycache__")).unwrap();

    let found = categories(&root, &ProviderSet::all());

    assert_eq!(
        found,
        vec![Category::PyCache],
        "only the cache is claimed: not the environments, not build, not dist"
    );
}

/// A nested artifact is claimed where the ecosystem puts it and nowhere else.
#[test]
fn a_nested_artifact_is_claimed_only_at_its_own_path() {
    let root = fixture();

    fs::create_dir_all(root.join("app/ios")).unwrap();
    fs::create_dir_all(root.join("app/ios/Pods")).unwrap();
    fs::write(root.join("app/package.json"), b"{}").unwrap();

    // The same name at the project root, where React Native never puts it.
    fs::create_dir_all(root.join("app/Pods")).unwrap();

    let found = categories(&root, &ProviderSet::all());

    assert!(found.contains(&Category::ReactNative));
    assert_eq!(
        found
            .iter()
            .filter(|c| **c == Category::ReactNative)
            .count(),
        1,
        "the bare Pods at the root is not React Native's"
    );
    assert!(
        !found.contains(&Category::CocoapodsPods),
        "no Podfile, so CocoaPods claims nothing either"
    );
}
