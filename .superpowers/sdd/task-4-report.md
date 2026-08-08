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
- Windows path replacement through an open retained directory returns `ERROR_ACCESS_DENIED`; post-open Windows coverage asserts that native refusal and then verifies secure deletion. Unix/macOS post-open synchronization coverage remains deterministic and sleep-free.

## Verification

- `cargo test --target-dir target-task4-final -j 1`: passed, 20 unit tests, 39 integration tests, 2 provider tests, doc tests.
- `cargo test --target-dir target-task4-final -j 1 --test node_modules`: passed, 39 tests.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --target-dir target-task4-final --all-targets --all-features -- -D warnings`: passed.

## Concerns

- Windows cannot rename retained target through its path while secure handles are open; the review regression documents this native behavior.
- Secure sizing remains partial until backend-specific verified-handle sizing exists.

## Review Fixes

- Restored policy-first precedence in `validate_clean_target`: provider/workspace, basename, context, and safety policy now run before captured/current identity comparison.
- Extended the test-only post-acquisition synchronization seam to Windows.
- Added Windows regression coverage that attempts same-basename replacement after `open_secure_target` returns, asserts native `ERROR_ACCESS_DENIED` while retained handles are open, then verifies handle-based deletion removes the original target and does not delete a replacement.
- No path-based production deletion, Trash support, or injected helper scope changes were added.

## Review Verification

- `cargo test --target-dir C:\Users\srmat\AppData\Local\Temp\opencode\uncached-core-task4-review -j 1`: passed, 21 unit tests, 40 integration tests, 2 provider tests, doc tests.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --target-dir C:\Users\srmat\AppData\Local\Temp\opencode\uncached-core-task4-review --all-targets --all-features -- -D warnings`: passed.

## Review Concerns

- On Windows, native sharing semantics reject same-basename replacement with `ERROR_ACCESS_DENIED` while secure handles remain open; test documents and verifies this behavior.
- Policy-first validation intentionally reports `target does not exist` before identity comparison when the selected path is missing.

## Task 4 Review Fixes

### Changes

- Added private `CleanPolicy` validation result retaining canonical workspace root and canonical target path.
- Public permanent and Trash cleanup now pass canonical root and a canonical target clone to `open_secure_target`; `validate_clean_target` still returns `CoreResult<PathBuf>`.
- Added aliased-root public-entry coverage using `root/project/..` for both cleanup APIs.
- Strengthened Windows replacement coverage: same-basename rename must return `ERROR_ACCESS_DENIED` while secure handles remain open; after secure handle deletion releases them, a distinct replacement directory and content are created at the original path and verified to survive.

### Verification

- TDD red: aliased-root tests failed with `SecureTargetOpenFailed` / non-typed Trash error before the canonical-root fix.
- Focused green: `cargo test --target-dir C:\Users\srmat\AppData\Local\Temp\opencode\uncached-core-task4-review-fix-green2 --lib canonical_workspace_root` passed, 2 tests.
- Windows regression: `cargo test --target-dir C:\Users\srmat\AppData\Local\Temp\opencode\uncached-core-task4-review-fix-windows --lib replacement_created_after_open_handles_are_released_survives_cleanup` passed.
- Full isolated suite: `cargo test --target-dir C:\Users\srmat\AppData\Local\Temp\opencode\uncached-core-task4-review-fix-full2 -j 1` passed, 24 unit tests, 40 integration tests, 2 provider tests, doc tests.
- `cargo fmt --all -- --check` passed.
- `cargo clippy --target-dir C:\Users\srmat\AppData\Local\Temp\opencode\uncached-core-task4-review-fix-clippy --all-targets --all-features -- -D warnings` passed.

### Concerns

- Windows test executables in repository-local target directories remain susceptible to MSVC linker locks; isolated target directories were used for verification.
