# Windows Identity Task 2 Report

Status: DONE

## Scope

- Worked only in `uncached-core`.
- Implemented Windows `file_identity` lookup with native `FILE_ID_INFO`.
- Did not implement fail-closed cleanup, retries, hashing, traversal changes, or cleanup policy changes.

## Implementation

- Opens the requested path with `CreateFileW` using `FILE_FLAG_BACKUP_SEMANTICS` for directories.
- Uses `FILE_FLAG_OPEN_REPARSE_POINT` so the open does not follow reparse points.
- Shares read, write, and delete access to avoid unnecessarily rejecting concurrently used directories.
- Reads `FILE_ATTRIBUTE_TAG_INFO` first and returns `None` for API failure, non-directory targets, or reparse-point targets.
- Reads `FILE_ID_INFO` with `GetFileInformationByHandleEx`.
- Copies `VolumeSerialNumber` and all 16 bytes of `FileId.Identifier` into `FileIdentity` without hashing or truncation.
- Uses an internal RAII handle guard that calls `CloseHandle` on every return path.
- Added Windows-only tests for stable identity on a real temporary directory and missing-directory failure.
- Added the `Win32_Security` feature required by `windows-sys` 0.59's `CreateFileW` binding.

## TDD Evidence

- RED: `cargo test file_identity` failed on Windows because the real-directory test received `None` from the unimplemented branch.
- GREEN: the same focused command passed after the native lookup was implemented.

## Verification

- `cargo test`: 42 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed.
- Runtime Windows verification was available and passed; Windows-only tests executed rather than being skipped.

## Self-Review

- Handle lifetime covers attribute and identity API calls and closes through `Drop`.
- Reparse-point safety is enforced both at open time and by rejecting the returned reparse attribute.
- Non-directory targets return `None`.
- Existing Unix identity mapping and tests remain unchanged in behavior.
- No unrelated files or policies were modified.

## Concerns

- No known concerns within Task 2 scope.
- Fail-closed cleanup remains intentionally deferred to Task 3.
