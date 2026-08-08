# Provider-Aware Scanning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move active Desktop provider discovery and Trash cleanup into `uncached-core` while keeping the free CLI focused on `node_modules`.

**Architecture:** Core owns shared categories, scanner matching, traversal, target construction, and category-aware Trash validation. Desktop supplies configured scanner implementations through a core `ProviderSet`, keeps UI-specific target IDs, asynchronous sizing, registry, and permanent deletion. CLI continues calling `ProviderSet::free()`, which remains NodeModules-only.

**Tech Stack:** Rust, Cargo, `jwalk`, `serde`, Tauri, existing Desktop scanner/settings/event layers.

## Global Constraints

- Active categories this cycle: `NodeModules`, `CargoTarget`, `NextJs`, `Turbo`, `PyCache`, `Gradle`.
- `DotNetObj` and `Docker` remain declared categories without detectors.
- `ProviderSet::free()` remains NodeModules-only for CLI.
- No Windows `FILE_ID_INFO` implementation in this cycle.
- No typed/serialized error redesign in this cycle.
- No permanent-deletion migration in this cycle.
- No traversal parallelism change in this cycle.
- Preserve existing Desktop working-tree changes outside touched feature files.
- Production behavior changes require a failing test first, then minimal implementation.

---

### Task 1: Expand Core Provider Contract

**Repository:** `uncached-core`

**Files:**
- Modify: `src/model.rs`
- Modify: `src/scanner.rs`
- Modify: `src/lib.rs`
- Test: `tests/node_modules.rs`

**Interfaces:**
- `Category` gains `CargoTarget`, `NextJs`, `Turbo`, `PyCache`, `Gradle`, `DotNetObj`, `Docker`, and `Custom(String)` while retaining `NodeModules`.
- `ProviderSet` exposes `scanner_for(category: Category) -> Option<&dyn Scanner>` or equivalent borrowed lookup without exposing its internal vector.
- `Scanner` keeps `category()`, `matches()`, and `confirm_context()` as the minimum provider contract.
- `ProviderSet::free() -> ProviderSet` remains exactly one `NodeModulesScanner`.

- [ ] **Step 1: Write failing tests for multi-category provider lookup.**

Add a test scanner with category `Category::Custom("fixture".to_owned())`, add it to a `ProviderSet`, assert lookup succeeds for an equal category and fails for an absent category. Add compile/test coverage that `ProviderSet::free()` still has one scanner with `Category::NodeModules`.

- [ ] **Step 2: Run the focused test and verify RED.**

Run: `cargo test --test node_modules custom_provider_lookup -- --exact`

Expected: compilation or test failure because expanded categories and category lookup are not implemented.

- [ ] **Step 3: Implement the minimal shared category and lookup API.**

Extend the serializable core enum and add a read-only category lookup method. Preserve `ProviderSet` ownership and existing scanner matching behavior. Do not add provider-specific detection rules yet.

- [ ] **Step 4: Run focused and full core tests.**

Run: `cargo test --test node_modules && cargo test`

Expected: all tests pass.

- [ ] **Step 5: Commit.**

```bash
git add src/model.rs src/scanner.rs src/lib.rs tests/node_modules.rs
git commit -m "feat(core): expand provider categories"
```

### Task 2: Make Core Scan All Configured Categories

**Repository:** `uncached-core`

**Files:**
- Modify: `src/scanner.rs`
- Test: `tests/node_modules.rs`

**Interfaces:**
- `scan_workspace(root, providers, cancelled) -> CoreResult<ScanOutcome>` returns every categorized provider target.
- `scan_workspace_with_sizing(root, providers, cancelled, size)` preserves category and identity for every target.
- `traverse_workspace` continues returning `TraversalEntry` and allows callback-driven `Prune`, `Continue`, and `Cancel`.

- [ ] **Step 1: Add a failing multi-provider scan test.**

Create two fixture directories with distinct marker files and two scanners matching `node_modules` and `custom-target`, returning different categories. Call `scan_workspace_with_sizing` with a deterministic size closure and assert both targets, categories, byte totals, sorted paths, and pruning behavior.

- [ ] **Step 2: Run the test and verify RED.**

Run: `cargo test --test node_modules scans_all_configured_provider_categories -- --exact`

Expected: failure because `scan_workspace_with_sizing` currently only collects `Category::NodeModules`.

- [ ] **Step 3: Generalize candidate collection.**

Collect `(PathBuf, Category)` for every `TraversalEntry` with `Some(category)`, prune accepted targets, sort deterministically by path, size each path, and construct `ScanTarget` with the discovered category and `file_identity`.

- [ ] **Step 4: Run core tests and clippy.**

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`

Expected: all tests pass with no clippy warnings.

- [ ] **Step 5: Commit.**

```bash
git add src/scanner.rs tests/node_modules.rs
git commit -m "feat(core): scan all configured providers"
```

### Task 3: Add Provider-Aware Core Trash Validation

**Repository:** `uncached-core`

**Files:**
- Modify: `src/cleaner.rs`
- Modify: `src/lib.rs`
- Test: `tests/node_modules.rs`

**Interfaces:**
- `validate_clean_target(root, providers, target) -> CoreResult<PathBuf>` resolves target category through `ProviderSet` and validates provider context.
- `clean_to_trash(root, providers, target) -> CoreResult<CleanReport>` delegates to the provider-aware validator.
- Test helper variants accept the same provider set so deletion remains injectable and deterministic.

- [ ] **Step 1: Add failing tests for non-NodeModules cleanup and missing providers.**

Define a fixture scanner for `custom-target` with a regular marker. Assert a matching target validates and is passed to injected delete; assert the same target returns `InvalidTarget` with an empty provider set; assert a mismatched context is rejected and delete is not called.

- [ ] **Step 2: Run focused tests and verify RED.**

Run: `cargo test --test node_modules cleanup_uses_matching_provider -- --exact`

Expected: compilation failure because cleanup functions do not accept providers.

- [ ] **Step 3: Implement provider-aware validation.**

Resolve scanner by `target.category`, reject missing provider, retain shared checks for identity, real directory, symlinked ancestors, canonical path, root containment, and safe deletion, then call `scanner.confirm_context` on the canonical target. Remove hard-coded NodeModules name/package checks from the shared path. Keep `NodeModulesScanner` context rules as its provider behavior.

- [ ] **Step 4: Update existing core cleanup call sites and run tests.**

Pass `ProviderSet::free()` to existing NodeModules tests and CLI-facing APIs. Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`

Expected: all core tests pass with no warnings.

- [ ] **Step 5: Commit.**

```bash
git add src/cleaner.rs src/lib.rs tests/node_modules.rs
git commit -m "feat(core): validate cleanup by provider"
```

### Task 4: Implement Desktop Providers Against Core Contract

**Repository:** `uncached-desktop`

**Files:**
- Modify: `src-tauri/src/scanner/mod.rs`
- Modify: `src-tauri/src/scanner/node_modules.rs`
- Modify: `src-tauri/src/scanner/rust.rs`
- Modify: `src-tauri/src/scanner/next.rs`
- Modify: `src-tauri/src/scanner/turbo.rs`
- Modify: `src-tauri/src/scanner/python.rs`
- Modify: `src-tauri/src/scanner/gradle.rs`
- Modify: `src-tauri/src/commands/settings.rs`
- Test: `src-tauri/src/scanner/mod.rs`

**Interfaces:**
- `get_default_providers(settings) -> uncached_core::ProviderSet` returns active providers selected by settings.
- Each Desktop provider implements `uncached_core::Scanner` directly or through a thin adapter; matching and context rules remain equivalent to current behavior.
- Settings flags map to NodeModules, CargoTarget, Gradle, NextJs, and Turbo; PyCache remains always registered as today.

- [ ] **Step 1: Add failing provider-set tests.**

Assert default settings register six active providers, each disabled setting removes only its provider, and provider categories are present exactly once.

- [ ] **Step 2: Run Desktop scanner tests and verify RED.**

Run: `cargo test --manifest-path src-tauri/Cargo.toml scanner::tests`

Expected: failure because `get_default_providers` and the core provider implementations do not exist.

- [ ] **Step 3: Replace Desktop-only scanner trait with core-compatible providers.**

Keep thin local constructors only where needed for settings. Move no new detection behavior into scan service. Preserve existing context predicates, including package markers and Cargo.toml checks.

- [ ] **Step 4: Run scanner tests and check compilation.**

Run: `cargo test --manifest-path src-tauri/Cargo.toml scanner::tests && cargo check --manifest-path src-tauri/Cargo.toml`

Expected: tests pass and Desktop compiles against the published core API after the dependency revision is updated at the integration step.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/scanner src-tauri/src/commands/settings.rs
git commit -m "feat(desktop): register core scan providers"
```

### Task 5: Migrate Desktop Scan Service to Core Scan

**Repository:** `uncached-desktop`

**Files:**
- Modify: `src-tauri/src/services/scan_service.rs`
- Modify: `src-tauri/src/models/target.rs`
- Test: `src-tauri/src/services/scan_service.rs`

**Interfaces:**
- Scan worker invokes `uncached_core::traverse_workspace` or `scan_workspace_with_sizing` with configured providers, never `ProviderSet::free()` for Desktop.
- A conversion helper maps `uncached_core::ScanTarget` to UI `ScanTarget`, preserving path, category, size, identity, and pending state semantics.
- Existing `scan:started`, `scan:progress`, `scan:target_found`, `scan:target_sized`, `scan:finished`, and `scan:cancelled` event shapes remain stable.

- [ ] **Step 1: Add failing conversion and provider-routing tests.**

Construct a core target for each active category and assert conversion preserves category/path/identity. Add a test seam around provider construction or a focused worker helper proving configured providers, not `ProviderSet::free()`, are passed to traversal.

- [ ] **Step 2: Run focused tests and verify RED.**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::scan_service::tests`

Expected: failure because conversion/provider routing is still NodeModules-specific.

- [ ] **Step 3: Replace NodeModules-only scan branching.**

Use `get_default_providers(&settings)` in the worker, accept every categorized `TraversalEntry`, create a pending UI target with the core category and identity, register it, emit it, start existing asynchronous sizing, and prune it. Preserve cancellation/session locking behavior.

- [ ] **Step 4: Verify Desktop scan behavior.**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo check --manifest-path src-tauri/Cargo.toml`

Expected: all tests pass and compilation succeeds.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/services/scan_service.rs src-tauri/src/models/target.rs
git commit -m "feat(desktop): route scans through core providers"
```

### Task 6: Migrate Desktop Trash Cleanup for All Active Categories

**Repository:** `uncached-desktop`

**Files:**
- Modify: `src-tauri/src/services/clean_service.rs`
- Test: `src-tauri/src/services/clean_service.rs`

**Interfaces:**
- `CleanService::clean_registered_target` reconstructs a core `ScanTarget` for every active category.
- Trash mode calls `uncached_core::clean_to_trash(&root, &providers, &target)` with the same configured provider set used by Desktop scanning.
- Permanent mode continues using `PermanentCleaner` and existing confirmation semantics.

- [ ] **Step 1: Add failing tests for non-NodeModules core cleanup.**

Use a CargoTarget or custom provider fixture and assert Trash cleanup performs core provider validation. Assert invalid context leaves the target intact. Keep existing NodeModules and Desktop-only regression tests.

- [ ] **Step 2: Run focused tests and verify RED.**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::clean_service::tests`

Expected: failure because cleanup currently uses core only for NodeModules and Desktop cleaners for all other categories.

- [ ] **Step 3: Delegate active-category Trash cleanup to core.**

Build the configured provider set from current settings or a shared provider factory, map the UI target to core including identity, call core cleanup, and retain UI size-error aggregation. Do not add a fallback provider for missing categories.

- [ ] **Step 4: Verify cleanup and full Desktop suite.**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo check --manifest-path src-tauri/Cargo.toml`

Expected: all tests pass and compilation succeeds.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/services/clean_service.rs
git commit -m "feat(desktop): use core cleanup providers"
```

### Task 7: Repin Consumers, Verify Workspace, and Publish

**Repositories:** `uncached-core`, `uncached-cli`, `uncached-desktop`

**Files:**
- Modify: `uncached-cli/Cargo.toml` and `uncached-cli/Cargo.lock` with the new core revision.
- Modify: `uncached-desktop/src-tauri/Cargo.toml` and `uncached-desktop/src-tauri/Cargo.lock` with the new core revision.
- Modify: `uncached-core/docs/superpowers/specs/2026-08-07-provider-aware-scanning.md` only if implementation reveals a concrete contract correction.

- [ ] **Step 1: Run independent verification before publishing.**

Run in core: `cargo fmt --check && cargo test && cargo clippy --all-targets --all-features -- -D warnings`.

Run in CLI: `cargo test && cargo check`.

Run in Desktop: `cargo test --manifest-path src-tauri/Cargo.toml && cargo check --manifest-path src-tauri/Cargo.toml`.

- [ ] **Step 2: Commit core implementation and publish it.**

Inspect `git status`, `git diff`, and `git log --oneline -10`; stage only intended core files and push the feature commits to `uncached-core/main`.

- [ ] **Step 3: Repin CLI and verify product scope.**

Update CLI to the published core revision, regenerate `Cargo.lock`, run CLI tests, and confirm its slogan/behavior remains NodeModules-only through `ProviderSet::free()`.

- [ ] **Step 4: Repin Desktop and verify product scope.**

Update Desktop to the same published core revision, regenerate `Cargo.lock`, run Desktop tests/check, and confirm all six active providers route through core.

- [ ] **Step 5: Review final diffs and push consumers.**

Confirm unrelated pre-existing Desktop changes remain uncommitted or separately committed. Run `git status --short` in all repositories, inspect final diffs, then push CLI `main` and Desktop `master`.
