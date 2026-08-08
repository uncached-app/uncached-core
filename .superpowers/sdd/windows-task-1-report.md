# Windows Identity Task 1 Report

Status: DONE

## Scope

- Worked only in `uncached-core`.
- Implemented identity model and dependency setup required by Task 1.
- Did not implement Windows API identity lookup or cleanup policy.
- Did not modify Desktop frontend payloads.

## Implementation

- Changed `FileIdentity` from `{ first, second }` to `{ volume_serial: u64, file_id: [u8; 16] }`.
- Preserved `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Serialize`, and `Deserialize` derives.
- Updated Unix `file_identity` mapping:
  - `metadata.dev()` maps to `volume_serial`.
  - `metadata.ino().to_le_bytes()` fills `file_id[0..8]`.
  - `file_id[8..16]` remains zero.
- Added target-specific `windows-sys` dependency with `Win32_Foundation` and `Win32_Storage_FileSystem` features.
- Added `serde_json` as a dev dependency for serialization tests.
- Added tests for JSON round trip, volume serial mismatch, file ID mismatch, and Unix metadata mapping.

## TDD Evidence

RED was established before production implementation. `cargo test` failed because the new tests referenced missing `volume_serial` and `file_id` fields on the old `FileIdentity`.

GREEN was established after the model and Unix mapping changes. All tests passed.

## Verification

- `cargo test`: 40 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `git diff --check`: passed before commit.

## Files

- `Cargo.toml`
- `Cargo.lock`
- `src/model.rs`
- `src/filesystem.rs`
- `.superpowers/sdd/windows-task-1-report.md`

## Concerns

- Windows `file_identity` lookup remains intentionally `None`; Windows API lookup is deferred to a later task as required.
- Unix mapping test is target-gated and therefore executes only on Unix targets.
