# Windows Trash via IFileOperation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable Windows Recycle Bin cleanup through `IFileOperation` with COM STA lifecycle, strong revalidation, typed errors, and no false-success results.

**Architecture:** Keep permanent deletion handle-based and unchanged. Implement Windows Trash as a separate Shell backend that creates `IShellItem`/`IFileOperation` on a dedicated STA thread, uses Recycle Bin flags, and validates identity/policy before and postconditions after the Shell operation.

**Tech Stack:** Rust, `windows-sys`, Win32 COM, Shell32 `IFileOperation`, Windows filesystem handles, Serde/CoreErrorInfo.

## Global Constraints

- Do not replace permanent handle-based deletion.
- Do not use `SHFileOperation` legacy path-string API.
- Do not add path-based fallback when Shell operation is unavailable.
- Do not expose HRESULT/native error text publicly.
- Do not claim Trash has identical atomicity to handle-based permanent delete.
- COM initialization and teardown are RAII-scoped.
- `IFileOperation` runs only on an STA thread.
- Revalidate policy, provider context, identity, symlink/reparse state before Shell operation.
- Revalidate postconditions before returning success.
- Any ambiguity returns a typed error and never reports successful cleanup.
- Permanent deletion remains independent of Trash capability.

---

## File Structure

- Modify: `uncached-core/Cargo.toml` — Windows COM/Shell feature flags
- Modify: `uncached-core/src/error.rs` — typed Trash errors and public codes
- Modify: `uncached-core/src/secure_cleanup.rs` — capability mapping and dispatch
- Modify: `uncached-core/src/secure_cleanup/windows.rs` — STA Shell backend
- Modify: `uncached-core/src/contract/v1.rs` — serialize new public errors
- Modify: `uncached-core/tests/contract_v1.rs` — error/capability contract tests
- Modify: `uncached-core/src/secure_cleanup/windows.rs` tests — Windows runtime fixtures
- Modify: `uncached-core/src/secure_cleanup.rs` tests — capability/unsupported behavior
- Modify: `uncached-core/CONTRACT_V1.md` — new error codes and capability semantics

---

### Task 1: Add Typed Trash Errors and Windows Dependencies

**Files:**
- Modify: `uncached-core/Cargo.toml`
- Modify: `uncached-core/src/error.rs`
- Modify: `uncached-core/src/contract/v1.rs`
- Modify: `uncached-core/src/secure_cleanup.rs`
- Modify: `uncached-core/tests/contract_v1.rs`

**Interfaces:**
- Produces new stable errors: `SecureTrashInitializationFailed`, `SecureTrashOperationFailed`, `SecureTrashAborted`, `SecureTrashPostconditionFailed`.
- Preserves `SecureTrashUnavailable`, `TargetIdentityChanged`, and `TargetReparsePoint`.
- Keeps public `CoreErrorInfo` sanitization and snake_case codes.

- [ ] **Step 1: Add failing error contract tests**

Extend `contract_v1.rs` with cases for every new error code and assert exact snake_case values, sanitized messages, nullable path/operation fields, and no HRESULT/native text.

- [ ] **Step 2: Run focused tests to verify RED**

```bash
cargo test --test contract_v1
```

Expected: compile failure because new variants/codes do not exist.

- [ ] **Step 3: Add Windows dependency features**

Extend the Windows `windows-sys` feature list with the exact modules needed by COM/Shell activation, including Foundation, System Com, System Ole, UI Shell, and Storage FileSystem. Keep features target-gated under `cfg(windows)`.

- [ ] **Step 4: Implement typed errors**

Add variants carrying internal `std::io::Error`/HRESULT context only where needed, then map them in `public_info()` to stable messages:

```rust
SecureTrashInitializationFailed
SecureTrashOperationFailed { path: PathBuf, operation: String, source: io::Error }
SecureTrashAborted { path: PathBuf }
SecureTrashPostconditionFailed { path: PathBuf }
```

Keep native source attached for logs but absent from `CoreErrorInfo.message`.

- [ ] **Step 5: Map capability**

Keep `move_to_trash` false until Windows backend implementation completes. Do not advertise a capability merely because Windows is compiled.

- [ ] **Step 6: Run GREEN checks**

```bash
cargo test --test contract_v1
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git commit -m "feat(core): add typed Windows Trash errors"
```

---

### Task 2: Implement COM STA IFileOperation Backend

**Files:**
- Modify: `uncached-core/src/secure_cleanup/windows.rs`
- Modify: `uncached-core/src/secure_cleanup.rs`

**Interfaces:**
- Consumes: `WindowsTarget`, verified target handles/identity, typed errors from Task 1.
- Produces: `WindowsTarget::move_to_trash() -> CoreResult<()>` and `CleanupCapabilities { trash: true }` when initialization/backend is available.

- [ ] **Step 1: Add failing backend tests**

Add Windows-only tests that call the backend and currently assert `SecureTrashUnavailable`. Keep fixtures under `%TEMP%` and assert target remains intact on current stub.

- [ ] **Step 2: Run RED on Windows**

```bash
cargo test secure_cleanup::windows
```

Expected: new test fails after changing expected behavior, or existing stub test identifies the implementation target.

- [ ] **Step 3: Add STA RAII wrapper**

Implement an internal `ComApartment` guard:

```rust
struct ComApartment;

impl ComApartment {
    fn initialize_sta() -> CoreResult<Self>;
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // CoUninitialize on same STA thread.
    }
}
```

Map failed `CoInitializeEx(COINIT_APARTMENTTHREADED)` to `SecureTrashInitializationFailed`. No COM pointer may cross worker/thread boundaries.

- [ ] **Step 4: Create Shell item and IFileOperation**

On the STA thread:

1. Convert validated target path to NUL-terminated UTF-16.
2. Call `SHCreateItemFromParsingName` to obtain `IShellItem`.
3. Call `CoCreateInstance` for `CLSID_FileOperation`.
4. Set silent operation flags with `FOF_ALLOWUNDO` and no confirmation UI.
5. Queue `DeleteItem(shell_item, null)`, then call `PerformOperations`.

Map each failed HRESULT to typed `SecureTrashOperationFailed` with operation name. Do not include HRESULT text in public message.

- [ ] **Step 5: Check abort state and success**

After `PerformOperations`, call `GetAnyOperationsAborted`. Map true to `SecureTrashAborted`; do not return success.

- [ ] **Step 6: Set capability only after backend path exists**

Update Windows `capabilities()` to report `trash: true` only for the implemented backend. Initialization/runtime failures remain typed operation errors, not capability lies.

- [ ] **Step 7: Run GREEN Windows checks**

```bash
cargo test secure_cleanup::windows
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git commit -m "feat(core): move Windows targets to Recycle Bin"
```

---

### Task 3: Add Strong Revalidation and Postcondition Checks

**Files:**
- Modify: `uncached-core/src/secure_cleanup/windows.rs`
- Modify: `uncached-core/src/cleaner.rs`
- Modify: `uncached-core/src/error.rs`

**Interfaces:**
- Consumes: existing `open_secure_target` handle/identity verification and Task 2 Shell operation.
- Produces: no false success under identity/reparse/replacement ambiguity.

- [ ] **Step 1: Add failing race/revalidation tests**

Add Windows fixtures for:

- replacement before Shell item creation;
- reparse/junction target and ancestor;
- target identity mismatch;
- target absent/ambiguous after Shell operation.

Each must assert typed error and preservation of replacement/non-target data.

- [ ] **Step 2: Revalidate immediately before Shell operation**

Before creating `IShellItem`, re-run existing policy/provider/name checks and compare current target identity with captured identity. Reject `TargetIdentityChanged` or `TargetReparsePoint` before Shell dispatch.

- [ ] **Step 3: Validate postcondition**

After `PerformOperations`, inspect the original workspace path and identity. Return success only when the original target is absent and no replacement can be confused with it. Return `SecureTrashPostconditionFailed` for ambiguous state.

- [ ] **Step 4: Preserve permanent delete behavior**

Run existing handle-based permanent deletion tests and assert Trash implementation does not share or replace the permanent delete path.

- [ ] **Step 5: Run GREEN tests**

```bash
cargo test secure_cleanup::windows
cargo test cleaner
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/secure_cleanup/windows.rs src/cleaner.rs src/error.rs
git commit -m "fix(core): harden Windows Trash revalidation"
```

---

### Task 4: Contract and Capability Integration Tests

**Files:**
- Modify: `uncached-core/tests/contract_v1.rs`
- Modify: `uncached-core/src/secure_cleanup/windows.rs` tests
- Modify: `uncached-core/src/secure_cleanup.rs` tests
- Modify: `uncached-core/CONTRACT_V1.md`

**Interfaces:**
- Consumes: completed Windows backend and typed errors from Tasks 1-3.
- Produces: stable public capability/error evidence and platform documentation.

- [ ] **Step 1: Add capability tests**

On Windows, assert capability reports `moveToTrash == true` when backend is compiled and available. On unsupported platforms, assert false. Do not make Linux/macOS tests depend on Windows Shell.

- [ ] **Step 2: Add error serialization tests**

Serialize every new error through `CoreErrorInfo` and assert stable codes/messages/path behavior with native HRESULT detail absent.

- [ ] **Step 3: Add non-mutation tests**

Force unavailable/aborted/postcondition error paths and assert target contents and replacements remain unchanged.

- [ ] **Step 4: Document security distinction**

Update `CONTRACT_V1.md` to state:

- permanent cleanup is handle-based and stronger;
- Windows Trash uses Shell `IFileOperation` with revalidation;
- Trash has a residual Shell namespace race;
- ambiguous results fail closed;
- `moveToTrash` capability does not guarantee every target operation succeeds.

- [ ] **Step 5: Run full Windows verification**

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo check
```

- [ ] **Step 6: Commit**

```bash
git add tests src/secure_cleanup.rs src/secure_cleanup/windows.rs CONTRACT_V1.md
git commit -m "test(core): verify Windows Trash contract"
```

---

### Task 5: Final Cross-Platform Verification

**Files:**
- Modify: `uncached-core/CONTRACT_V1.md` only for measured platform results
- Modify: `uncached-desktop/docs/SECURITY.md` if Desktop-facing capability/security wording needs update

**Interfaces:**
- Consumes: completed Windows Trash backend.
- Produces: final evidence without changing CLI or Desktop IPC shape.

- [ ] **Step 1: Run Windows runtime suite**

```bash
cargo test
```

Record Recycle Bin success, abort, identity, reparse, postcondition, and permanent-delete results.

- [ ] **Step 2: Run Linux/macOS compile/runtime checks**

```bash
cargo check --target x86_64-unknown-linux-gnu
cargo check --target x86_64-apple-darwin
```

Run native runtime tests in VMs where available. Confirm `moveToTrash=false` outside supported Windows backend.

- [ ] **Step 3: Inspect security diff**

```bash
git status --short
git diff --check
git diff --stat
```

Confirm no `SHFileOperation`, path-based fallback, permanent-delete regression, or public native HRESULT detail.

- [ ] **Step 4: Commit documentation and push**

```bash
git add CONTRACT_V1.md
git commit -m "docs(core): document Windows Trash capability"
git push origin main
```
