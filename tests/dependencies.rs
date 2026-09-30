use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use uncached_core::{analyse_manifest, ConfidenceLevel, UnusedReason};

static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

/// The `bin` and peer fields of the published `expo` 54 and `@astrojs/check`
/// manifests, which are what the rules below read.
const EXPO: &str = r#"{
  "name": "expo",
  "bin": { "expo": "bin/cli", "expo-modules-autolinking": "bin/autolinking", "fingerprint": "bin/fingerprint" },
  "peerDependencies": { "@expo/dom-webview": "*", "@expo/metro-runtime": "*", "react": "*", "react-native": "*", "react-dom": "*", "react-native-web": "*", "react-native-webview": "*" },
  "peerDependenciesMeta": { "@expo/dom-webview": { "optional": true }, "@expo/metro-runtime": { "optional": true }, "react-dom": { "optional": true }, "react-native-web": { "optional": true }, "react-native-webview": { "optional": true } }
}"#;
const ASTRO_CHECK: &str = r#"{
  "name": "@astrojs/check",
  "bin": { "astro-check": "./bin/astro-check.js" },
  "peerDependencies": { "typescript": "^5.0.0 || ^6.0.0" }
}"#;
const PLAIN: &str = r#"{ "name": "plain" }"#;

fn project(files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "uncached-core-dependencies-{}-{}",
        std::process::id(),
        FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for (path, contents) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    root
}

fn installed(name: &str, manifest: &str) -> (String, String) {
    (
        format!("node_modules/{name}/package.json"),
        manifest.to_owned(),
    )
}

fn unused(manifest: &Path) -> Vec<(String, ConfidenceLevel, UnusedReason)> {
    analyse_manifest(manifest)
        .unwrap()
        .unused
        .into_iter()
        .map(|dependency| (dependency.name, dependency.confidence, dependency.reason))
        .collect()
}

fn node_project(package: &str, files: &[(&str, &str)], installs: &[(&str, &str)]) -> PathBuf {
    let installs: Vec<(String, String)> = installs
        .iter()
        .map(|(name, manifest)| installed(name, manifest))
        .collect();
    let mut all: Vec<(&str, &str)> = vec![("package.json", package)];
    all.extend_from_slice(files);
    all.extend(
        installs
            .iter()
            .map(|(path, manifest)| (path.as_str(), manifest.as_str())),
    );
    project(&all).join("package.json")
}

#[test]
fn an_installed_package_nothing_names_is_reported_with_high_confidence() {
    let manifest = node_project(
        r#"{ "dependencies": { "expo-status-bar": "~3.0.0", "zod": "^4" } }"#,
        &[("app/index.tsx", r#"import { z } from "zod";"#)],
        &[("expo-status-bar", PLAIN), ("zod", PLAIN)],
    );
    assert_eq!(
        unused(&manifest),
        [(
            "expo-status-bar".into(),
            ConfidenceLevel::High,
            UnusedReason::NotReferenced
        )]
    );
}

#[test]
fn subpaths_and_runtime_specifiers_name_their_package() {
    let manifest = node_project(
        r#"{ "dependencies": { "react-dom": "*", "@scope/lib": "*", "stripe": "*", "bootstrap": "*" } }"#,
        &[
            (
                "src/main.tsx",
                r#"import { createRoot } from 'react-dom/client'; import x from "@scope/lib/sub";"#,
            ),
            (
                "supabase/functions/pay.ts",
                r#"import Stripe from 'npm:stripe@22.5.0';"#,
            ),
            ("src/app.scss", r#"@import "~bootstrap/scss/bootstrap";"#),
        ],
        &[
            ("react-dom", PLAIN),
            ("@scope/lib", PLAIN),
            ("stripe", PLAIN),
            ("bootstrap", PLAIN),
        ],
    );
    assert_eq!(unused(&manifest), []);
}

#[test]
fn an_optional_peer_of_an_installed_package_is_in_use() {
    let manifest = node_project(
        r#"{ "dependencies": { "expo": "~54", "react-native-web": "^0.21" } }"#,
        &[("app.json", r#"{ "expo": { "name": "app" } }"#)],
        &[("expo", EXPO), ("react-native-web", PLAIN)],
    );
    assert_eq!(unused(&manifest), []);
}

#[test]
fn a_tool_with_a_config_file_is_in_use() {
    let manifest = node_project(
        r#"{ "devDependencies": { "postcss": "^8", "@tailwindcss/postcss": "^4" } }"#,
        &[(
            "postcss.config.mjs",
            r#"export default { plugins: { "@tailwindcss/postcss": {} } };"#,
        )],
        &[("postcss", PLAIN), ("@tailwindcss/postcss", PLAIN)],
    );
    assert_eq!(unused(&manifest), []);
}

#[test]
fn a_package_another_tools_subcommand_loads_is_in_use_only_when_that_runs() {
    let package = |script: &str| {
        format!(
            r#"{{ "scripts": {{ "check": "{script}" }}, "devDependencies": {{ "@astrojs/check": "^0.9" }} }}"#
        )
    };
    let runs = node_project(
        &package("astro check && astro build"),
        &[],
        &[("@astrojs/check", ASTRO_CHECK)],
    );
    assert_eq!(unused(&runs), []);

    let never_runs = node_project(
        &package("astro build"),
        &[],
        &[("@astrojs/check", ASTRO_CHECK)],
    );
    assert_eq!(
        unused(&never_runs),
        [(
            "@astrojs/check".into(),
            ConfidenceLevel::Medium,
            UnusedReason::CommandOnly
        )]
    );
}

#[test]
fn a_command_a_script_or_workflow_runs_is_in_use() {
    let typescript =
        r#"{ "name": "typescript", "bin": { "tsc": "./bin/tsc", "tsserver": "./bin/tsserver" } }"#;
    let wrangler = r#"{ "name": "wrangler", "bin": { "wrangler": "./bin/wrangler.js" } }"#;
    let manifest = node_project(
        r#"{ "scripts": { "check": "tsc --noEmit" }, "devDependencies": { "typescript": "^5", "wrangler": "^4" } }"#,
        &[(
            ".github/workflows/deploy.yml",
            "steps:\n  - run: pnpm exec wrangler deploy\n",
        )],
        &[("typescript", typescript), ("wrangler", wrangler)],
    );
    assert_eq!(unused(&manifest), []);
}

#[test]
fn doubt_lowers_confidence_instead_of_hiding_the_finding() {
    let manifest = node_project(
        r#"{ "devDependencies": { "eslint-plugin-react": "*", "@types/lodash": "*", "left-pad": "*" } }"#,
        &[],
        &[("eslint-plugin-react", PLAIN), ("@types/lodash", PLAIN)],
    );
    let mut found = unused(&manifest);
    found.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        found,
        [
            (
                "@types/lodash".into(),
                ConfidenceLevel::Review,
                UnusedReason::TypesOnly
            ),
            (
                "eslint-plugin-react".into(),
                ConfidenceLevel::Medium,
                UnusedReason::ConventionLoaded
            ),
            (
                "left-pad".into(),
                ConfidenceLevel::Medium,
                UnusedReason::NotInstalled
            ),
        ]
    );
}

#[test]
fn types_for_a_package_in_use_are_in_use() {
    let manifest = node_project(
        r#"{ "dependencies": { "lodash": "*" }, "devDependencies": { "@types/lodash": "*", "@types/babel__core": "*", "@babel/core": "*" } }"#,
        &[(
            "index.js",
            r#"const _ = require("lodash"); import "@babel/core";"#,
        )],
        &[
            ("lodash", PLAIN),
            ("@types/lodash", PLAIN),
            ("@types/babel__core", PLAIN),
            ("@babel/core", PLAIN),
        ],
    );
    assert_eq!(unused(&manifest), []);
}

#[test]
fn a_nested_package_is_its_own_scope_unless_the_root_declares_workspaces() {
    let files: &[(&str, &str)] = &[
        ("packages/app/package.json", "{}"),
        ("packages/app/index.js", r#"import "zod";"#),
    ];
    let standalone = node_project(
        r#"{ "dependencies": { "zod": "*" } }"#,
        files,
        &[("zod", PLAIN)],
    );
    assert_eq!(
        unused(&standalone),
        [(
            "zod".into(),
            ConfidenceLevel::High,
            UnusedReason::NotReferenced
        )]
    );

    let workspace = node_project(
        r#"{ "workspaces": ["packages/*"], "dependencies": { "zod": "*" } }"#,
        files,
        &[("zod", PLAIN)],
    );
    assert_eq!(unused(&workspace), []);
}

#[test]
fn a_nested_packages_declarations_do_not_count_as_references() {
    let manifest = node_project(
        r#"{ "workspaces": ["packages/*"], "devDependencies": { "zod": "*" } }"#,
        &[(
            "packages/app/package.json",
            r#"{ "dependencies": { "zod": "*" } }"#,
        )],
        &[("zod", PLAIN)],
    );
    assert_eq!(
        unused(&manifest),
        [(
            "zod".into(),
            ConfidenceLevel::High,
            UnusedReason::NotReferenced
        )]
    );
}

fn cargo_project(manifest: &str, files: &[(&str, &str)]) -> PathBuf {
    let mut all: Vec<(&str, &str)> = vec![("Cargo.toml", manifest)];
    all.extend_from_slice(files);
    project(&all).join("Cargo.toml")
}

#[test]
fn a_crate_no_rust_file_refers_to_is_reported_with_high_confidence() {
    let manifest = cargo_project(
        "[package]\nname = \"app\"\n\n[dependencies]\njwalk = \"0.8\"\ntrash = \"5\"\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json.workspace = true\n\n[dev-dependencies]\ntempfile = \"3\"\n",
        &[
            ("src/main.rs", "use serde::Deserialize;\nfn main() { let trash = 1; }\n"),
            ("tests/it.rs", "#[test] fn t() { let _ = tempfile::tempdir(); }\n"),
        ],
    );
    let mut found = unused(&manifest);
    found.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        found,
        [
            (
                "jwalk".into(),
                ConfidenceLevel::High,
                UnusedReason::NotReferenced
            ),
            (
                "serde_json".into(),
                ConfidenceLevel::High,
                UnusedReason::NotReferenced
            ),
            (
                "trash".into(),
                ConfidenceLevel::High,
                UnusedReason::NotReferenced
            ),
        ]
    );
}

#[test]
fn renamed_featured_and_ignored_crates_are_not_reported_as_unused() {
    let manifest = cargo_project(
        r#"[package]
name = "app"

[package.metadata.cargo-machete]
ignored = ["prost"]

[dependencies]
tokio-util = "0.7"
fancy = { package = "fancy-regex", version = "0.14" }
prost = "0.13"
serde = { version = "1", optional = true }
chrono = { version = "0.4", optional = true }

[target.'cfg(unix)'.dependencies]
libc = "0.2"

[features]
serde = ["dep:serde", "chrono?/serde"]
"#,
        &[("src/lib.rs", "use tokio_util::sync::CancellationToken;\nfn f() { fancy::Regex::new(\"a\"); libc::getpid(); }\n")],
    );
    assert_eq!(unused(&manifest), []);
}

#[test]
fn a_crate_that_may_exist_for_its_features_or_linkage_is_medium() {
    let manifest = cargo_project(
        "[package]\nname = \"app\"\n\n[dependencies]\nopenssl = { version = \"0.10\", features = [\"vendored\"] }\nopenssl-sys = \"0.9\"\n",
        &[("src/main.rs", "fn main() {}\n")],
    );
    let mut found = unused(&manifest);
    found.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        found,
        [
            (
                "openssl".into(),
                ConfidenceLevel::Medium,
                UnusedReason::FeaturesOrLinkage
            ),
            (
                "openssl-sys".into(),
                ConfidenceLevel::Medium,
                UnusedReason::FeaturesOrLinkage
            ),
        ]
    );
}

#[test]
fn a_member_crates_code_does_not_count_for_the_crate_above_it() {
    let manifest = cargo_project(
        "[package]\nname = \"root\"\n\n[dependencies]\nregex = \"1\"\n",
        &[
            ("src/main.rs", "fn main() {}\n"),
            ("crates/member/Cargo.toml", "[package]\nname = \"member\"\n"),
            ("crates/member/src/lib.rs", "use regex::Regex;\n"),
        ],
    );
    assert_eq!(
        unused(&manifest),
        [(
            "regex".into(),
            ConfidenceLevel::High,
            UnusedReason::NotReferenced
        )]
    );
}

#[test]
fn a_virtual_workspace_root_declares_nothing_to_report() {
    let manifest = cargo_project(
        "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.dependencies]\nserde = \"1\"\n",
        &[],
    );
    let report = analyse_manifest(&manifest).unwrap();
    assert_eq!((report.declared, report.unused.len()), (0, 0));
}

fn removal_commands(manifest: &Path) -> Vec<String> {
    let mut commands: Vec<String> = analyse_manifest(manifest)
        .unwrap()
        .unused
        .into_iter()
        .map(|dependency| dependency.remove_with)
        .collect();
    commands.sort();
    commands
}

#[test]
fn the_removal_command_is_the_projects_own_package_manager() {
    let by_lockfile = node_project(
        r#"{ "dependencies": { "left-pad": "*" } }"#,
        &[("pnpm-lock.yaml", "lockfileVersion: '9.0'\n")],
        &[],
    );
    assert_eq!(removal_commands(&by_lockfile), ["pnpm remove left-pad"]);

    let by_field = node_project(
        r#"{ "packageManager": "yarn@4.5.0", "dependencies": { "left-pad": "*" } }"#,
        &[],
        &[],
    );
    assert_eq!(removal_commands(&by_field), ["yarn remove left-pad"]);

    let by_default = node_project(r#"{ "dependencies": { "left-pad": "*" } }"#, &[], &[]);
    assert_eq!(removal_commands(&by_default), ["npm uninstall left-pad"]);
}

#[test]
fn cargo_remove_is_told_the_section_and_platform() {
    let manifest = cargo_project(
        "[package]\nname = \"app\"\n\n[dependencies]\nregex = \"1\"\n\n[dev-dependencies]\ntempfile = \"3\"\n\n[build-dependencies]\ncc = \"1\"\n\n[target.'cfg(unix)'.dependencies]\nnix = \"0.29\"\n",
        &[("src/main.rs", "fn main() {}\n")],
    );
    assert_eq!(
        removal_commands(&manifest),
        [
            "cargo remove --build cc",
            "cargo remove --dev tempfile",
            "cargo remove --target 'cfg(unix)' nix",
            "cargo remove regex",
        ]
    );
}

#[test]
fn a_project_is_every_manifest_inside_it_but_not_installed_packages() {
    let root = project(&[
        ("package.json", r#"{ "workspaces": ["apps/*"] }"#),
        (
            "apps/web/package.json",
            r#"{ "dependencies": { "zod": "*" } }"#,
        ),
        ("src-tauri/Cargo.toml", "[package]\nname = \"app\"\n"),
        ("node_modules/zod/package.json", r#"{ "name": "zod" }"#),
        ("broken/package.json", "{ not json"),
    ]);
    let found = uncached_core::analyse_project(&root);
    let manifests: Vec<_> = found
        .reports
        .iter()
        .map(|report| report.manifest.strip_prefix(&root).unwrap().to_path_buf())
        .collect();
    assert_eq!(
        manifests,
        [
            PathBuf::from("apps/web/package.json"),
            PathBuf::from("package.json"),
            PathBuf::from("src-tauri/Cargo.toml"),
        ]
    );
    assert_eq!(found.unreadable, [root.join("broken/package.json")]);
}
