# Windows Identity Task 3 Report

Status: DONE

## Scope

- Worked only in `uncached-core`.
- Updated cleanup validation to fail closed on Windows when identity
  evidence is missing, unreadable, or changed.
- Preserved Unix optional identity semantics and existing cleanup error types,
  deletion paths, traversal, listing, sizing, provider, basename, context,
  root, symlink, and safe-directory checks.

## Implementation

- Windows `validate_clean_target` now requires `ScanTarget.identity`.
- Windows validation reads current identity with `file_identity` and maps
  lookup failure to `CoreError::InvalidTarget`.
- Windows captured/current identity mismatches remain
  `CoreError::InvalidTarget("target filesystem identity changed")`.
- Validation still runs before sizing and injected/native deletion closures;
  the existing second validation remains in place.
- Existing positive cleanup fixtures now provide scanner-equivalent identities
  on Windows; rejection fixtures remain identity-free where that is the case
  under test.

## TDD Evidence

- RED: Windows missing-captured-identity test failed before implementation.
- GREEN: focused missing identity, unreadable current identity, and
  cross-platform mismatch tests passed after implementation.

## Verification

- `cargo test`: 45 passed, 0 failed, including Windows-only runtime tests.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed.
- Windows runtime verification was available and passed.

## Self-Review

- Unix branch retains prior conditional `(Some(expected), Some(actual))`
  behavior, including optional identity semantics.
- Windows missing captured identity and current lookup failure return the same
  existing `InvalidTarget` variant before target metadata or deletion.
- Provider lookup remains first; all existing safety checks remain mandatory.
- No native deletion handles, traversal, listing, sizing, or error model were
  changed.
- No unrelated files were modified.

## Concerns

- No known concerns within Task 3 scope.

## Final Review Fix: Parent Reparse Traversal

Status: DONE

### Root Cause

- `FILE_FLAG_OPEN_REPARSE_POINT` applied to the final `CreateFileW` handle does not stop Windows from traversing a parent junction/reparse point first.
- Existing `file_identity` therefore captured identity for a target reached through a parent junction.

### Fix

- Windows `file_identity` now checks every existing path ancestor before opening the target.
- Each component is checked with `symlink_metadata` and `GetFileAttributesW`; any reparse attribute, failed metadata lookup, or failed native attribute lookup returns `None`.
- Final-handle reparse validation remains unchanged.
- Valid normal paths and Unix behavior remain unchanged.
- No consumers were modified.

### Regression Test

- Added Windows-only `file_identity_returns_none_when_parent_is_a_reparse_point`.
- Test creates a real Windows junction with `cmd /C mklink /J`, verifies identity returns `None`, and cleans up the fixture.
- RED: `cargo test file_identity_returns_none_when_parent_is_a_reparse_point -- --nocapture` failed before the fix with `Some(FileIdentity { ... })` instead of `None`.
- GREEN: same command passed after the fix and created a real junction.

### Verification

- `cargo test file_identity -- --nocapture`: passed, 5 passed, 0 failed; included real junction regression.
- `cargo test`: passed, 46 passed, 0 failed; Windows runtime tests available and passed.
- `cargo fmt --all`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed.

### Concerns

- The ancestor validation is a pre-open check and cannot eliminate all filesystem mutation races between validation and handle acquisition; it addresses parent reparse traversal as required without changing consumers or deletion behavior.

## Follow-Up Review Fixes

Status: DONE

### Finding 1: Ancestor Enumeration

- Replaced `path.ancestors()` error classification with progressive `Path::components()` enumeration.
- Only real `Normal` filesystem components are queried. Prefix and root namespace components are preserved without being treated as ordinary filesystem entries, covering UNC and extended Windows paths.
- Successful `symlink_metadata` reporting a symlink, or successful `GetFileAttributesW` reporting `FILE_ATTRIBUTE_REPARSE_POINT`, returns `None`.
- Metadata and native attribute query failures are not interpreted as reparse evidence; final target open and final handle attribute checks retain fail-closed identity behavior.

### Finding 2: Junction Test Setup

- Junction creation failure now removes the real fixture and panics with the `mklink /J` exit status.
- Test cannot silently pass without exercising parent-junction protection.
- Junction creation succeeded during Windows runtime verification.

### Verification

- RED baseline: prior implementation returned target identity through the parent junction; regression assertion failed with `Some(FileIdentity { ... })` instead of `None`.
- `cargo test file_identity`: passed, 5 passed, 0 failed.
- `cargo test`: passed, 46 passed, 0 failed; Windows runtime tests available and passed.
- `cargo fmt --all`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed.

### Concerns

- Namespace components are intentionally not queried as ordinary filesystem ancestors because UNC/extended namespace roots may not support normal metadata lookup. Final target opening and handle validation remain fail-closed.
- Pre-open validation cannot eliminate filesystem mutation races between component checks and handle acquisition.

## Follow-Up Review Fix: Fail Closed on Ancestor Query Errors

Status: DONE

### Fix

- A real `Normal` component whose `symlink_metadata` lookup fails now immediately returns `None`.
- A real component whose `GetFileAttributesW` lookup returns `INVALID_FILE_ATTRIBUTES` now immediately returns `None`.
- Only `Prefix` and `RootDir` namespace components remain excluded from ordinary component validation, preserving UNC and extended path handling.
- Junction protection, final target reparse checks, Windows cleanup fail-closed behavior, and Unix behavior are unchanged.

### Verification

- `cargo test file_identity`: passed, 5 passed, 0 failed; junction runtime test succeeded.
- `cargo test`: passed, 46 passed, 0 failed; Windows runtime tests available and passed.
- `cargo fmt --all`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed.

### Concerns

- Namespace prefix/root components remain intentionally unqueried; real filesystem components fail closed on either metadata or native attribute lookup failure.
- Pre-open validation cannot eliminate filesystem mutation races between component checks and handle acquisition.

## Follow-Up Review Fix: Cross-Platform Identity Fail-Closed

Status: DONE

### Policy

- Cleanup validation now requires captured and current filesystem identities on every supported platform.
- Missing captured identity, unreadable current identity, and identity mismatch all return the existing `CoreError::InvalidTarget` errors before sizing or deletion.
- Unix identity remains device plus inode; Windows identity remains volume serial plus full `FILE_ID_128`.

### Implementation

- Removed the Unix-only optional `(Some(expected), Some(actual))` comparison.
- Kept provider lookup, path canonicalization, provider context, containment, symlink, and safe-directory validation unchanged.
- Kept existing sizing, second-validation, deletion, and native-handle boundaries unchanged.

### Regression Tests

- Cross-platform missing captured identity rejects before injected sizing and deletion.
- Cross-platform unreadable current identity rejects before injected sizing and deletion.
- Cross-platform mismatched identity rejects before injected sizing and deletion.
- Existing Windows junction and final reparse identity tests remain enabled.

### Verification

- Windows runtime tests are available; run results recorded after implementation below.
- `cargo test`: passed, 46 passed, 0 failed; Windows junction runtime test succeeded.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed.

### Concerns

- Unix/macOS runtime was not available in this Windows environment; cross-platform
  rejection coverage is compiled into the shared test suite and Windows execution
  passed.
