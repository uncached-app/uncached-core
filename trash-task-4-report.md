# Trash Task 4 Report

## Status

Implementation complete from HEAD `89645c4`; final commit follows verification.

## Changes

- Added raw COM `IFileOperationProgressSink` implementation with `Advise` and `Unadvise` lifecycle.
- Captured `PostDeleteItem` HRESULT and expected-target correspondence.
- Required successful target callback, no Shell abort, and existing postcondition checks before reporting Trash success.
- External deletion without callback, wrong-target callback, failed callback, and ambiguous postconditions return `SecureTrashPostconditionFailed`.
- Added deterministic test seam for `SecureTrashAborted`; verified target contents remain unchanged.
- Preserved permanent handle-based deletion and approved residual Shell namespace race policy.
- Reclassified Windows `SecureTrashUnavailable` as explicit environment/backend unavailability in runtime tests; other Windows errors fail tests.
- Documented Windows-only `moveToTrash` capability, callback evidence, and residual race in `CONTRACT_V1.md`.

## TDD

- RED: evidence tests failed to compile before evidence classification existed.
- GREEN: evidence tests passed after implementation.
- RED debugging: first real Shell run exposed COM access violation; `ProgressSink` lacked `#[repr(C)]`.
- GREEN: focused real Windows Shell Trash test passed after correcting COM layout.

## Verification

- Focused Windows evidence tests: passed.
- Focused Windows real Trash test: passed in isolated target.
- Full `cargo test --target-dir %TEMP%\\uncached-core-task4-final2 -j 1`: passed, 56 unit tests, 22 contract tests, 42 node_modules tests, 5 provider tests, doc tests.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --target-dir %TEMP%\\uncached-core-task4-clippy2 --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed.

## Concerns

- Shell callback evidence reduces but cannot eliminate the documented path/Shell namespace race; postcondition ambiguity remains fail-closed.
- Windows runtime Trash coverage depends on Shell/Recycling Bin availability. `SecureTrashUnavailable` is reported explicitly and preserves target; operation failures are not skipped.
- Isolated Cargo target directories may be needed if MSVC test artifacts are locked by another process.

## ProgressSink Release Race Fix

### Changes

- Changed `progress_sink_release` to derive remaining references from one `fetch_sub(1, Ordering::AcqRel) - 1` result.
- Removed the post-decrement `ProgressSink` access that could read `references` after another concurrent `Release` freed the object.
- Preserved COM ABI, vtable wiring, final `Box::from_raw` free, and returned reference counts.
- Added a Windows-only concurrent final-release regression test.

### Verification

- TDD RED: concurrent release test failed on current implementation because non-final `Release` could return an invalid count after final free.
- TDD GREEN: focused concurrent release test passed.
- Windows focused suite: 29 passed.
- Full `cargo test`: 57 unit tests, 22 contract tests, 42 node_modules tests, 5 provider tests, doc tests passed.
- `cargo fmt --all -- --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed.

### Scope

- Core-only changes: `src/secure_cleanup/windows.rs` and this report.
- No Desktop, CLI, config, or lockfile changes.
