# Task 4 Report

## Status

Implemented in commit `5375020`.

## Changes

- Public permanent cleanup now uses policy validation, one `open_secure_target`, and consumed secure deletion.
- Public Trash cleanup uses secure setup and typed `SecureTrashUnavailable`; no Trash support was added.
- Removed production path-based deletion and path-based Trash calls from public entry points.
- Preserved injected delete helpers for policy/error tests; production entry points do not call them.
- Preserved `validate_clean_target` identity/error behavior while separating policy validation before secure open.
- Returned existing partial-size shape (`freed_bytes: 0`, `size_has_errors: true`) because current secure backends lack secure sizing.
- Added public cleanup regression coverage, typed Trash refusal coverage, and cross-platform provider expectations.

## TDD

- Wrote replacement regression before routing implementation.
- Initial execution hit Windows linker/object locks; fresh isolated target compiled and ran verification.
- Windows path replacement through an open retained directory returns `ERROR_ACCESS_DENIED`; Windows coverage uses public pre-open replacement rejection plus existing handle deletion tests. Unix/macOS post-open synchronization coverage remains deterministic and sleep-free.

## Verification

- `cargo test --target-dir target-task4-final -j 1`: passed, 20 unit tests, 39 integration tests, 2 provider tests, doc tests.
- `cargo test --target-dir target-task4-final -j 1 --test node_modules`: passed, 39 tests.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --target-dir target-task4-final --all-targets --all-features -- -D warnings`: passed.

## Concerns

- Windows cannot rename retained target through its path while secure handles are open, so post-open replacement coverage is Unix/macOS-only.
- Secure sizing remains partial until backend-specific verified-handle sizing exists.
