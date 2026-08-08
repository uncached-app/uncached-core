# Windows Trash via IFileOperation Design

## Goal

Enable safe Windows Recycle Bin cleanup through the Shell `IFileOperation`
COM API while preserving existing handle-based permanent deletion and the
fail-closed security model.

## Security boundary

Windows permanent deletion remains the strongest operation: it uses retained
verified handles and filesystem identities. Recycle Bin movement is a separate
capability because Windows Shell APIs operate through Shell items and do not
provide the same atomic handle-based deletion guarantee.

Trash is enabled only with strong revalidation and explicit documentation of
the residual Shell namespace race. Any ambiguity returns a typed error and
never reports successful cleanup.

## Backend API

Replace the Windows `move_to_trash` stub with a dedicated implementation:

```rust
impl WindowsTarget {
    pub(super) fn capabilities(&self) -> CleanupCapabilities;
    pub(super) fn delete_permanently(self) -> CoreResult<()>;
    pub(super) fn move_to_trash(self) -> CoreResult<()>;
}
```

`capabilities().trash` becomes true only when the Windows Shell backend is
available for the current operation. Permanent deletion remains independent.

## Shell operation

Use `IFileOperation`, which supersedes `SHFileOperation` and accepts Shell
items. Do not use the legacy path-string API.

Operation sequence:

1. Validate workspace, provider context, target basename, and expected identity.
2. Reopen/retain the verified target and ancestors using existing Windows
   handle-based code.
3. Create an `IShellItem` for the validated target.
4. Initialize COM on a dedicated single-threaded apartment (STA) thread with
   `CoInitializeEx(COINIT_APARTMENTTHREADED)`.
5. Create `IFileOperation` through COM activation.
6. Configure silent operation flags with Recycle Bin/undo enabled and no second
   native confirmation dialog.
7. Queue `DeleteItem` for the Shell item.
8. Call `PerformOperations`.
9. Check HRESULT and `GetAnyOperationsAborted`.
10. Revalidate postconditions before returning success.

The UI already confirms cleanup, so Explorer must not show a second confirmation
dialog. No COM interface or Shell object may cross thread boundaries.

## COM lifecycle

COM initialization and teardown are RAII-scoped:

- `CoInitializeEx(COINIT_APARTMENTTHREADED)` runs before Shell interfaces;
- failed initialization maps to `SecureTrashInitializationFailed`;
- `CoUninitialize` runs on every initialized path;
- `IFileOperation` runs only on its STA thread;
- `spawn_blocking` MTA workers must not call `IFileOperation` directly.

## Revalidation and postconditions

Before Shell operation:

- workspace containment is revalidated;
- target provider/category/context is revalidated;
- target and ancestors are rejected if symlink/reparse;
- current target identity matches captured identity;
- Shell item creation succeeds for the validated target.

After Shell operation:

- successful `PerformOperations` must not be treated as sufficient alone;
- the original workspace target must no longer be present;
- a replacement created during the operation must not be considered deleted;
- ambiguous absence/identity state maps to `SecureTrashPostconditionFailed`;
- no error path reports success.

This reduces, but cannot completely eliminate, the path/Shell namespace race
inherent to the Windows Recycle Bin API. Permanent deletion retains its stronger
handle-based guarantee.

## Public errors

Add stable public error codes and sanitized messages:

- `SecureTrashUnavailable` — Recycle Bin/backend unavailable;
- `SecureTrashInitializationFailed` — COM/Shell initialization failed;
- `SecureTrashOperationFailed` — Shell operation returned failure HRESULT;
- `SecureTrashAborted` — Shell reports operation aborted;
- `SecureTrashPostconditionFailed` — result cannot be proven safely;
- existing `TargetIdentityChanged` and `TargetReparsePoint` for revalidation.

HRESULTs and native error details remain internal logs. `CoreErrorInfo` exposes
stable code/message/path/operation only.

## Testing

Windows runtime fixtures under `%TEMP%` must cover:

- valid directory moves to Recycle Bin;
- target absent from workspace after success;
- files/directories inside target are included in Shell operation;
- identity replacement before operation is rejected;
- symlink/junction/reparse target or ancestor is rejected;
- Shell HRESULT maps to typed error;
- Shell abort maps to typed error;
- COM STA initializes and uninitializes on every path;
- unavailable Recycle Bin/backend preserves target without mutation;
- postcondition ambiguity never reports success;
- permanent deletion remains handle-based and independent of Trash.

Tests must validate workspace identity and non-mutation, not fixed Recycle Bin
filesystem paths or localized Explorer output.

Add capability tests proving `trash` is true only for the implemented Windows
backend and remains false on unsupported platforms.

## Non-goals

- Do not replace permanent handle-based deletion.
- Do not use `SHFileOperation` legacy path-string API.
- Do not add path-based fallback when Shell operation is unavailable.
- Do not expose HRESULT/native error text publicly.
- Do not claim Trash has identical atomicity to handle-based permanent delete.
