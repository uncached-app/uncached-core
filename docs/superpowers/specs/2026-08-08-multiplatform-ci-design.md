# Multiplatform CI Design

## Goal

Add explicit, reproducible CI to each component repository. CI must validate the existing Rust, frontend, contract, and platform-specific behavior on push, pull request, and manual dispatch without changing application behavior or publishing artifacts.

## Repository Boundaries

The workspace contains separate Git repositories:

- `uncached-core`: Rust library and Windows Trash implementation.
- `uncached-desktop`: Tauri Rust workspace plus frontend.
- `uncached-cli`: Node package and existing CLI tests.

Each repository owns its workflow under `.github/workflows/`. No root-level workflow is introduced because the parent directory is not a Git repository and does not receive component pushes.

## Trigger Policy

Every workflow runs on:

- `push` to the repository's primary branch (`main` for Core and CLI, `master` for Desktop);
- `pull_request` targeting the primary branch;
- `workflow_dispatch` for manual reruns.

Workflows use `permissions: contents: read` and do not require secrets.

## Core Workflow

Run one job using an OS matrix:

- `ubuntu-latest`;
- `windows-latest`;
- `macos-latest`.

Each matrix entry runs:

1. Stable Rust setup with dependency caching.
2. `cargo fmt --check`.
3. `cargo check --all-targets`.
4. `cargo test --all-targets`.
5. `cargo clippy --all-targets --all-features -- -D warnings`.

The pruning stress benchmark remains opt-in and does not run in normal CI. Windows executes the normal platform tests on a Windows runner. A runtime `SecureTrashUnavailable` result remains a typed, fail-closed behavior; CI must not convert it into success or silently skip unexpected failures.

## Desktop Workflows

Desktop uses two independent validation jobs.

### Desktop Rust

Run on the same Linux, Windows, and macOS matrix. Install stable Rust with caching, then execute formatting, check, tests, and clippy using the existing workspace commands. The job does not invoke a full Tauri bundle/build initially because native platform SDKs, bundler configuration, and artifact publication are separate concerns.

### Desktop Frontend

Run on Ubuntu with the repository's existing Node lockfile and scripts:

1. Install the repository-supported Node version.
2. Run `npm ci`.
3. Run `npm test`.
4. Run `npm run build`.

This job validates TypeScript, frontend tests, and production bundling without starting a browser or Tauri runtime.

## CLI Workflow

Run the existing Node test suite only. Use the repository's supported Node version and lockfile with `npm ci`, then run `npm test`. This CI change does not migrate CLI behavior to contract v1 and does not add package publishing or native binary builds.

## Caching And Lockfiles

Use official setup actions and their supported dependency-cache mechanisms. CI must use lockfiles as committed. Commands must not update or rewrite `Cargo.lock` or npm lockfiles. No generated artifacts are uploaded.

## Failure Semantics

- Any command failure fails its job.
- Platform-specific tests are not replaced by compile-only checks on their native OS.
- Unsupported optional runtime behavior must be represented by the existing typed contract; broad `continue-on-error` is forbidden.
- Missing platform tooling is a workflow configuration failure, not a reason to hide test failures.

## Out Of Scope

- CLI contract v1 migration.
- Full Tauri packaging or release artifacts.
- npm publishing.
- Stress benchmark execution on every CI run.
- New runtime abstractions or application behavior.

## Acceptance Criteria

- All three repositories contain explicit workflows with correct primary branches and triggers.
- Core and Desktop Rust validate on Linux, Windows, and macOS.
- Desktop frontend tests and build pass in CI.
- CLI existing tests pass in CI.
- Workflows use read-only repository permissions and no secrets.
- Normal CI does not mutate lockfiles or publish artifacts.
- Workflow YAML is syntactically valid and commands match repository scripts.
