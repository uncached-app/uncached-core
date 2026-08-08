use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use uncached_core::{scan_workspace, Category, ProviderSet, ScanOutcome};

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("uncached-core-providers-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn categories(root: &Path, providers: &ProviderSet) -> Vec<Category> {
    let ScanOutcome::Completed(report) =
        scan_workspace(root, providers, &AtomicBool::new(false)).unwrap()
    else {
        panic!("scan unexpectedly cancelled");
    };
    report
        .targets
        .into_iter()
        .map(|target| target.category)
        .collect()
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
        .scanner_for(Category::NodeModules)
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
