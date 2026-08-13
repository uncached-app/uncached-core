use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use uncached_core::{traverse_workspace, Category, ProviderSet, TraversalControl};

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
fn all_registers_six_scanners_in_deterministic_order() {
    let providers = ProviderSet::all();
    assert_eq!(providers.len(), 6);

    let expected = [
        Category::NodeModules,
        Category::CargoTarget,
        Category::NextJs,
        Category::Turbo,
        Category::PyCache,
        Category::Gradle,
    ];
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
    .map(|(path, category)| (fs::canonicalize(root.join(path)).unwrap(), category))
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
