# Task 2 Report: Unix FD Backend

## Status

Implemented Unix secure target opening and fail-closed destructive dispatch.

## RED Evidence

Added focused Unix tests before backend implementation in `src/secure_cleanup/unix.rs`:

- target identity mismatch returns `TargetIdentityChanged`
- direct symlink target returns `TargetReparsePoint`
- symlinked ancestor returns `TargetReparsePoint`
- unsupported top-level deletion returns `SecureOperationUnsupported` without mutation

The requested focused command was run on the Windows host:

```text
cargo test secure_cleanup::unix::tests -- --nocapture
running 0 tests
```

Windows excludes the Unix module, so runtime RED could not be observed locally. A Linux target was installed and the Unix library compiled with `cargo check`, but test linking failed because Linux linker `cc` is unavailable. No Unix test result is claimed as executed.

## GREEN Evidence

Unix backend compilation:

```text
cargo check --target x86_64-unknown-linux-gnu
Finished `dev` profile
```

Windows verification:

```text
cargo test
50 passed, 0 failed
cargo fmt --all -- --check
passed
```

`git diff --check` passed.

## Files

- `Cargo.toml`: add direct `libc` dependency for Unix native wrappers.
- `Cargo.lock`: lock `libc` for `uncached-core`.
- `src/secure_cleanup.rs`: dispatch Unix backend while preserving unsupported non-Unix behavior.
- `src/secure_cleanup/unix.rs`: RAII FD ownership, `openat` no-follow traversal, `fstat` identity conversion, typed errors, and native-operation wrappers.
- `.superpowers/sdd/task-2-report.md`: this report.

## Implementation Review

- Target and each relative ancestor are opened with `O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW`.
- Target identity comes from `fstat` on retained target FD and is compared with `ScanTarget.identity`.
- Workspace, parent, target FD, and basename remain owned by `UnixTarget`.
- Symlink/reparse rejection happens during FD-relative opening.
- No `canonicalize`, `/proc/self/fd`, reopened destructive path, `remove_dir_all`, or Trash fallback was added.
- Unix permanent deletion returns `SecureOperationUnsupported` before mutation because `unlinkat` cannot bind top-level directory removal to the retained target FD identity.
- Trash remains unavailable on Unix backend.
- No Windows implementation or cleaner integration was added.

## Concerns

- Unix runtime tests need a Linux/macOS runner with native linker available; this Windows environment only provided Unix cross-compilation.
- Top-level Unix permanent cleanup is intentionally unavailable until a platform primitive can prove identity-preserving removal. This is fail-closed but leaves cleanup unperformed.

## Review Fixes

- Replaced path-only workspace opening with component-by-component `openat` traversal from `/` or `.` using `O_NOFOLLOW`, including all workspace ancestors.
- Implemented FD-relative child enumeration with `readdir` on a duplicated directory FD.
- Implemented recursive child deletion using `fstatat(AT_SYMLINK_NOFOLLOW)`, `openat(O_NOFOLLOW)`, and `unlinkat`; symlink entries are rejected.
- Kept top-level directory deletion fail-closed and before mutation because Unix `unlinkat` cannot bind removal to the retained target identity.
- Added real `cfg(unix)` tests for identity mismatch, direct symlink, symlinked ancestor, unsupported top-level deletion, and nested FD-relative child removal.
- Documented unsafe native calls and ownership transfers, including valid descriptors, NUL-free names, initialized stat storage, and `OwnedFd`/`DIR*` ownership.

## Fix Verification

RED test attempt after adding Unix tests:

```text
cargo test --target x86_64-unknown-linux-gnu secure_cleanup::unix::tests -- --nocapture
error: linker `cc` not found
```

The Unix test binary could not link on this Windows host, so no Unix runtime result is claimed.

GREEN/build evidence:

```text
cargo check --target x86_64-unknown-linux-gnu
Finished `dev` profile

cargo test
50 passed, 0 failed

cargo fmt --all -- --check
passed
```

The focused Unix test command was rerun after implementation and failed at the same missing Linux linker boundary. `git diff --check` passed before commit.

## Revised Architecture Fixes

- Removed `unlinkat` wrappers and recursive destructive traversal. Unix backend now retains permanent capability `false` and returns `SecureOperationUnsupported` before any mutation.
- Kept recursive FD-relative traversal as validation-only; regular files are observed without opening them as directories, while symlinks and directory-open failures remain rejected.
- `readdir` now clears and checks thread-local errno, distinguishing EOF from enumeration failure and returning `SecureEnumerationFailed` on error.
- Retained real Unix tests for identity mismatch, direct symlink, symlinked ancestor, unsupported deletion with no mutation and false capabilities, plus nested validation traversal.

## Revised Verification

Windows host:

```text
cargo test
9 unit tests passed, 39 node_modules tests passed, 2 provider tests passed, 0 failed

cargo fmt --all -- --check
passed
```

Unix cross-check:

```text
cargo check --target x86_64-unknown-linux-gnu
Finished `dev` profile
```

Unix runtime tests remain unavailable on this Windows host because the Linux linker `cc` is not installed. No Unix runtime pass is claimed.

## Task 2 Architecture Correction

- Changed Unix dispatch to `cfg(all(unix, not(target_os = "macos")))`.
- Added separate `cfg(target_os = "macos")` dispatch and `src/secure_cleanup/macos.rs` backend.
- macOS backend uses its own Darwin `openat`/`fstat`/`fdopendir`/`readdir` wrappers, rejects symlinked entries, validates recursively, reports `SecureEnumerationFailed`, and exposes no permanent or Trash capability.
- Non-macOS Unix secure opening now invokes FD-relative validation before returning `UnixTarget`; permanent deletion remains unsupported and non-mutating.
- Added Unix regression coverage for symlink children rejected during secure open and macOS-gated identity, symlink, and unsupported-operation tests.

## Final Verification

Windows host:

```text
cargo test
9 unit tests passed, 39 node_modules tests passed, 2 provider tests passed, 0 failed

cargo fmt --all -- --check
passed
```

Linux cross-target:

```text
cargo check --target x86_64-unknown-linux-gnu
Finished `dev` profile
```

TDD regression execution:

```text
cargo test --target x86_64-unknown-linux-gnu secure_open_rejects_symlink_child_during_validation -- --nocapture
failed before test execution: linker `cc` not found
```

macOS:

```text
cargo check --target x86_64-apple-darwin
failed: target may not be installed (`can't find crate for core` / `std`)
```

macOS runtime is unavailable on this Windows host; no macOS runtime pass is claimed.

## Final Review Finding: Errno Support Matrix

- Restricted the non-macOS Unix backend dispatch to Linux and Android, where the
  `readdir` errno accessors are implemented with `__errno_location`.
- Preserved macOS as a separate backend using Darwin `__error` accessors.
- Other Unix targets now use the typed `Unsupported` backend instead of compiling
  `unix.rs`; they cannot mistake a `readdir` failure for EOF.
- Removed the generic no-op errno accessors. No unlinkat/path fallback or cleaner
  integration was added; public cleaner integration remains deferred to Task 4.

### Support Matrix

| Target family | Backend | Errno accessor | Permanent | Trash |
| --- | --- | --- | --- | --- |
| Linux | non-macOS Unix FD validation | `__errno_location` | unsupported | unavailable |
| Android | non-macOS Unix FD validation | `__errno_location` | unsupported | unavailable |
| macOS | separate macOS FD validation | `__error` | unsupported | unavailable |
| Other Unix | typed unsupported backend | not applicable | unsupported | unavailable |
| Non-Unix | typed unsupported backend | not applicable | unsupported | unavailable |

### Review Fix Verification

Windows host:

```text
cargo test
10 unit tests, 39 node_modules tests, 2 provider tests, 2 doc tests passed; 0 failed
```

Linux cross-target:

```text
cargo check --target x86_64-unknown-linux-gnu
Finished `dev` profile
```

The Linux runtime test binary remains unavailable because the Windows host has no
Linux linker. Formatting and diff checks were run after this review fix.
