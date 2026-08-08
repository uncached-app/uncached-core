# Task 6 Verification Report

Date: 2026-08-08
Host: Windows x86_64, `x86_64-pc-windows-msvc`
Implementation baseline: `82a16d2`

## Verification Commands

Commands ran sequentially from `uncached-core`. Windows commands used
`CARGO_TARGET_DIR=target-task6-final`.

| Command | Result | Evidence |
| --- | --- | --- |
| `cargo fmt --all -- --check` | PASS | Exit code 0; no output |
| `cargo check` | PASS | Exit code 0; `Finished dev profile` |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS | Exit code 0; no warnings |
| `cargo test` | PASS | 25 unit tests, 41 `node_modules` tests, 2 `providers` tests, 0 failures; doc-tests 0 tests |
| `cargo test --test secure_cleanup` | NOT APPLICABLE | Exit code 101: repository has no integration test target named `secure_cleanup`; secure-cleanup tests run as unit tests in `cargo test` |
| `cargo test --test node_modules` | PASS | 41 passed, 0 failed |
| `cargo check --target x86_64-unknown-linux-gnu` | PASS | Linux target installed; successful compile after isolated serial retry using `target-task6-linux-serial`, `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0` |

The first Linux cross-check attempts in `target-task6-final` and
`target-task6-linux` hit Windows file-lock errors while Cargo removed build
artifacts. The required isolated-target retry succeeded.

## Security Invariant Review

- No production `fs::remove_dir_all` or `trash::delete` authority found. Matches are test-fixture cleanup only, under `#[cfg(test)]` modules.
- Windows permanent deletion uses retained handles and `SetFileInformationByHandle` with delete disposition. No path reopen is used as destructive authority after secure acquisition.
- Non-macOS Unix and macOS backends report `permanent: false` and `trash: false`; permanent cleanup and Trash return typed unavailable errors.
- Trash is typed unavailable in every backend, including unsupported backend handling.
- Policy validation precedes identity validation in `validate_clean_target`; public secure cleanup validates policy before opening the secure target.
- Canonical workspace is passed into `open_secure_target`; canonical target is passed through `canonical_target`.
- Public replacement regression is non-vacuous: the test creates original content, replaces the same basename, runs public cleanup, asserts identity-change failure, and asserts both replacement and original content survive. Windows also exercises replacement while retained handles block path replacement.

## Platform Evidence

- Windows native unit and integration tests executed on this Windows host and passed.
- Linux native runtime tests were not run. `cargo check --target x86_64-unknown-linux-gnu` is compile-only; this Windows host did not provide a Linux native runner/linker.
- macOS runtime tests were not run and remain pending; no macOS runner was available.

## Concerns

- The brief names `cargo test --test secure_cleanup`, but no such Cargo integration target exists. This is a command/spec mismatch, not an implementation failure; secure-cleanup coverage is included in the passing unit suite.
- Cargo artifact locks occurred during Linux cross-check attempts on Windows. Serial isolated-target retry resolved verification.

## Final Whole-Branch Review Fix

Date: 2026-08-08

### Changes

- Replaced unchecked Windows `FILE_ID_EXTD_DIR_INFO` dereferences with a bounded parser using checked offsets and unaligned reads.
- Parser validates record header availability, field layout, even UTF-16 name length, name slice bounds, record bounds, and nonzero `NextEntryOffset` minimum, alignment, and in-buffer range. Malformed responses map to `SecureEnumerationFailed`.
- Added Windows-only pure-buffer tests for truncated headers, odd name lengths, invalid offsets, and out-of-bounds names.
- Mapped `ERROR_INVALID_FUNCTION`, `ERROR_NOT_SUPPORTED`, and `ERROR_INVALID_PARAMETER` from `FileDispositionInfoEx` to `SecureOperationUnsupported` while preserving source and path. Other native failures remain `SecureDeletionFailed`.
- Kept Windows permanent capability explicit; no fallback was added. Unix/macOS runtime policy, identity/reparse handling, UTF-16 conversion, and injected cleaner helpers remain unchanged.

### TDD and Verification

- RED: focused malformed-record test command initially failed to compile because parser and native-error mapping did not exist; after implementation, one test exposed an incorrect truncated-header fixture and was corrected.
- GREEN: `cargo test secure_cleanup::windows::tests -- --nocapture` passed, 16 tests.
- Full Windows suite: `cargo test -j 1` passed, 30 unit tests, 41 `node_modules` tests, 2 provider tests, 2 doc tests, 0 failures.
- `cargo clippy --all-targets --all-features -- -D warnings` passed.
- `cargo fmt --all -- --check` passed.
- `cargo check --target x86_64-unknown-linux-gnu` passed with isolated target and serial build settings.
- `git diff --check` passed.

### Concerns

- Linux and macOS runtime tests remain unavailable on this Windows host; Linux cross-check is compile-only.
- Existing repository-local `target-*` directories are unrelated untracked build artifacts and were not modified or removed.
