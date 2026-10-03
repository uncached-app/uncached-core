# Core Public Contract v1

This is the canonical serialized contract exposed by `uncached-core::contract::v1`. It covers envelopes, public errors, capabilities, and transport DTOs. Desktop event names and command behavior are documented in `uncached-desktop/docs/IPC.md`.

Desktop commands that use this public boundary return the same v1 envelope as
events. A command failure has `data: null` and sanitized `error` information;
native Rust error strings do not cross IPC.

## Envelope

Every serialized envelope has exactly four keys and uses camelCase:

```json
{
  "apiVersion": "v1",
  "platform": "windows",
  "data": {},
  "error": null
}
```

`apiVersion` is `"v1"` for values constructed with `ApiEnvelope::success` or `ApiEnvelope::failure`. Consumers can use `ApiEnvelope::is_supported_version()` before handling a decoded envelope. `platform` is one of:

```text
windows | linux | macos | android | unsupported
```

Success has non-null `data` and null `error`. Failure has null `data` and non-null `error`. Deserialization rejects envelopes with both or neither payload. The Core deserializer preserves the version string instead of deciding whether a consumer supports it; consumers must reject versions they do not implement. The Desktop v1 adapter rejects every version other than `v1`.

## DTOs

All fields below are serialized with the shown camelCase names.

### `CleanupCapabilities`

```json
{
  "permanentCleanup": true,
  "moveToTrash": true,
  "secureIdentity": true
}
```

`permanentCleanup` and `moveToTrash` report available cleanup backends. `secureIdentity` reports platform support for stable target identity checks. Values are capability facts, not authorization; callers must still handle backend rejection. Unix targets use the platform Trash integration supplied by the `trash` crate and revalidate the recorded filesystem identity immediately before cleanup.

Permanent cleanup is identity-checked immediately before deletion. On Windows, Trash uses Shell `IFileOperation` with revalidation before and after releasing verification handles. Success additionally requires an `IFileOperationProgressSink::PostDeleteItem` callback for the expected target with a successful HRESULT, no operation abort, and passing postcondition checks. External deletion without that callback returns `secure_trash_postcondition_failed`, not success. On Unix, permanent cleanup removes the tree through a pinned directory descriptor (`openat`/`unlinkat`) and re-checks the recorded identity before unlinking the target, so a replacement is rejected rather than removed. Unix Trash revalidates the recorded identity and then hands the path to the `trash` crate, so a replacement made between those two steps is not detected. Neither claims Windows handle-atomic guarantees. A cleanup capability of `true` means the backend is available, not that every target operation will succeed.

The following capability example is Windows-only:

```json
{
  "permanentCleanup": true,
  "moveToTrash": true,
  "secureIdentity": true
}
```

Linux and macOS report both cleanup capabilities as `true`. Android and unsupported platforms report both as `false`; `secure_trash_unavailable` is an explicit backend/environment result and must not be treated as successful cleanup. Capabilities are checked before a cleanup opens anything: on a platform without a backend the call fails with `secure_operation_unsupported` or `secure_trash_unavailable` without touching the target tree.

`secureIdentity` is `true` wherever Core can read a stable filesystem identity and has a supported secure cleanup backend. It is currently enabled for Windows, Linux, and macOS; Android and other unsupported targets report `false`.

### Measured platform verification

The CI workflow runs only when dispatched by hand (`workflow_dispatch`). Its
matrix runs formatting, Clippy, `cargo test --all-features` and benchmark
compilation on Linux and Windows; macOS is deliberately left out (see the
comment in `.github/workflows/ci.yml`), so macOS behaviour is verified locally.
The Windows
backend includes tests for Recycle Bin success and abort handling, target
identity and reparse-point rejection, callback and postcondition validation,
concurrent final-release handling, replacement preservation, and permanent
delete behavior. This document does not claim a local Windows execution; the
Windows results are established by the corresponding CI job.

### Secure open scope and limits

Permanent cleanup deletes through retained handles, so its open retains a handle
for every entry in the target subtree and verifies the whole tree before deleting
anything. Two bounds keep that fail-closed instead of exhausting process
resources: at most 500,000 entries (`secure_target_too_large`) and at most 512
directory levels (`secure_target_too_deep`). Collection is iterative, so nesting
depth cannot exhaust the stack, and it expands one directory level at a time
across a shared worker pool while keeping a deterministic parent-before-child
order.

Trash cleanup does not open the subtree. It hands a path to the Shell and
releases every handle before that call, so descendant handles could never guard
the operation. Only the target and its ancestors are pinned, and their identities
are revalidated immediately before and after the Shell boundary. The Shell moves
the target as a single item and does not traverse reparse points inside it. A
secure target opened for Trash refuses `delete_permanently` rather than deleting
a partial tree.

Access follows the same rule. A handle that will delete an entry is opened with
`DELETE`; a handle that only pins an entry and reads its identity — every
ancestor, the Trash target, and every revalidation — is opened for attribute
access only. This matters because `DELETE` participates in Windows share-mode
checks: requesting it fails with `secure_target_open_failed` and OS error 32
whenever another process holds the entry open without delete sharing, which is
the normal state of a build directory, an open editor, or a process's working
directory. Permanent cleanup still pays that price because the handle is how it
deletes; nothing else does.

### `ScanTargetDto`

```json
{
  "path": "C:\\workspace\\node_modules",
  "category": "NodeModules",
  "sizeBytes": 123,
  "sizePending": false,
  "sizeHasErrors": false
}
```

`FileIdentity` is deliberately absent. `category` is always a string, never an
object. Known values:

```text
NodeModules | CargoTarget | NextJs | Turbo | PyCache | Gradle | DotNetObj | Docker
```

Wire recognition does not imply default scanner availability. `ProviderSet::all`
currently registers scanners for `NodeModules`, `CargoTarget`, `NextJs`, `Turbo`,
`PyCache`, and `Gradle`; `DotNetObj` and `Docker` are reserved wire categories
until their scanners are implemented or explicitly configured by a consumer.

A custom category serializes as `"Custom:<name>"` — for example
`"Custom:docker-buildx"`. The name must be non-empty, contain no control
characters, and be at most 128 UTF-8 bytes. Any other string is rejected on
deserialization.

### Core `TargetFoundDto`

```json
{ "scanId": "scan-1", "target": { "path": "...", "category": "NodeModules", "sizeBytes": 123, "sizePending": true, "sizeHasErrors": false } }
```

This is Core's domain transport DTO. It does not describe every field in the
Desktop event payload.

`ScanTargetDto::from_settled` accepts only an explicit `SettledScanTarget` wrapper,
so pending targets cannot be passed accidentally. The wrapper carries the final
`sizeHasErrors` value; conversion emits `sizePending: false` and preserves that
error flag. Desktop scan events use their own DTO when they need to preserve live
sizing state.

### `TargetSizedDto`

```json
{
  "scanId": "scan-1",
  "targetId": "scan-1-abc123",
  "sizeBytes": 123,
  "sizePending": false,
  "sizeHasErrors": false,
  "formattedSize": "123 B"
}
```

### `ScanFinishedDto`

```json
{ "scanId": "scan-1", "totalTargets": 2, "totalBytesFound": 42, "durationMs": 7 }
```

### `ScanFailedDto`

```json
{
  "scanId": "scan-1",
  "error": { "code": "io", "message": "I/O error", "path": null, "operation": null }
}
```

`scan:failed` is a successful v1 envelope containing this data DTO. The nested
`scanId` correlates failure with its scan; the nested error remains sanitized.

### `CleanupResultDto`

```json
{
  "success": true,
  "targetPath": "C:\\workspace\\target",
  "freedBytes": 42,
  "sizePending": false,
  "sizeHasErrors": false
}
```

This Core DTO has exactly these five required fields. Desktop cleanup events
intentionally use a separate event DTO and may omit `targetPath`.

Secure cleanup does not walk the target after deletion. It reports the
`sizeBytes` measured during the settled scan as `freedBytes`, so a successful
cleanup normally returns `sizePending: false`. This is an estimated reclaimed
size based on the pre-cleanup scan, not a post-operation filesystem measurement.
`sizePending` means "not measured"; `sizeHasErrors` means "measured, and the
walk hit errors". Consumers must not treat `freedBytes` as an exact physical
disk-space delta when the scan had sizing errors.

## Errors

`error` has this exact public shape:

```json
{
  "code": "io",
  "message": "I/O error",
  "path": null,
  "operation": null
}
```

`code` is stable snake_case. Current codes:

```text
invalid_workspace
io
provider_unavailable
target_missing
target_not_directory
target_name_unsupported
target_provider_mismatch
target_context_invalid
target_outside_workspace
target_identity_unavailable
target_identity_changed
target_reparse_point
secure_operation_unsupported
secure_target_open_failed
secure_target_too_large
secure_target_too_deep
secure_enumeration_failed
secure_handle_query_failed
secure_deletion_failed
secure_trash_unavailable
secure_trash_initialization_failed
secure_trash_operation_failed
secure_trash_aborted
secure_trash_postcondition_failed
```

Every cleanup-policy rejection has its own code, so a consumer can tell "target is
gone" from "target left the workspace" without parsing a message. The removed
`invalid_target`, `trash` and `recycle_bin_too_large` codes had no producer left
once cleanup moved to the secure backends; consumers must reject unknown codes as
the version policy below requires.

`message` is a stable public summary. `path` and `operation` are required keys
whose values may be strings or null. Native error sources, private filesystem
details, and `FileIdentity` never serialize into `error`.

## Event names

Core defines DTOs; Desktop emits these names using v1 envelopes:

```text
scan:started
scan:target_found
scan:target_sized
scan:progress
scan:finished
scan:cancelled
scan:failed
clean:started
clean:finished
clean:failed
```

`scan:finished` carries `ScanFinishedDto`. `scan:failed` carries `ScanFailedDto`
inside `data`, not the envelope-level failure field. Core's `TargetFoundDto` is the
domain shape for `scan:target_found`; Desktop wraps that shape in its
Desktop-specific target-found DTO, adding `id`, `name`, `formattedSize`, and
nullable `lastModified`. `scan:target_sized` carries `TargetSizedDto`. Core
`CleanupResultDto` is the five-field command/domain result. Desktop
`clean:finished` uses a separate event DTO when it intentionally omits
`targetPath`. Cleanup failure events carry an error envelope with null data.
`clean:finished` is the shipped success name; `clean:success` is not part of
v1.

## Version and migration policy

Unknown versions are hard failures for v1 consumers. They must not be coerced, partially decoded, or treated as v1. A v2 must define an explicit versioned DTO/envelope and adapter, with compatibility decided by both producer and consumer. Do not add fields or semantics to v1 that require v1 consumers to understand v2 behavior.

The CLI is explicitly a current-shape consumer until its separate migration is implemented. Core Contract v1 documentation does not imply CLI v1 envelope support, and this contract task does not modify CLI.
