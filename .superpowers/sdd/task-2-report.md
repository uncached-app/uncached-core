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
