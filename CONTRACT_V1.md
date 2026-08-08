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

`apiVersion` is `"v1"` for values constructed with `ApiEnvelope::success` or `ApiEnvelope::failure`. `platform` is one of:

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
  "moveToTrash": false,
  "secureIdentity": true
}
```

`permanentCleanup` and `moveToTrash` report available cleanup backends. `secureIdentity` reports platform support for stable target identity checks. Values are capability facts, not authorization; callers must still handle backend rejection.

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

`FileIdentity` is deliberately absent. `category` is the Core `Category` enum: `NodeModules`, `CargoTarget`, `NextJs`, `Turbo`, `PyCache`, `Gradle`, `DotNetObj`, `Docker`, or `Custom(String)`.

### Core `TargetFoundDto`

```json
{ "scanId": "scan-1", "target": { "path": "...", "category": "NodeModules", "sizeBytes": 123, "sizePending": true, "sizeHasErrors": false } }
```

This is Core's domain transport DTO. It does not describe every field in the
Desktop event payload.

`ScanTargetDto::from_settled` is restricted to targets whose sizing lifecycle has
settled. Its `sizePending` and `sizeHasErrors` values are therefore both false.
Desktop scan events use their own DTO when they need to preserve live sizing
state.

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

### `CleanupResultDto`

```json
{
  "success": true,
  "targetPath": "C:\\workspace\\target",
  "freedBytes": 42,
  "sizeHasErrors": false
}
```

This Core DTO has exactly these four required fields. Desktop cleanup events
intentionally use a separate event DTO and may omit `targetPath`.

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
invalid_target
io
trash
recycle_bin_too_large
target_identity_unavailable
target_identity_changed
target_reparse_point
secure_operation_unsupported
secure_target_open_failed
secure_enumeration_failed
secure_handle_query_failed
secure_deletion_failed
secure_trash_unavailable
```

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

`scan:finished` carries `ScanFinishedDto`. Core's `TargetFoundDto` is the
domain shape for `scan:target_found`; Desktop wraps that shape in its
Desktop-specific target-found DTO, adding `id`, `name`, `formattedSize`, and
nullable `lastModified`. `scan:target_sized` carries `TargetSizedDto`. Core
`CleanupResultDto` is the four-field command/domain result. Desktop
`clean:finished` uses a separate event DTO when it intentionally omits
`targetPath`. Failure events carry an error envelope with null data.
`clean:finished` is the shipped success name; `clean:success` is not part of
v1.

## Version and migration policy

Unknown versions are hard failures for v1 consumers. They must not be coerced, partially decoded, or treated as v1. A v2 must define an explicit versioned DTO/envelope and adapter, with compatibility decided by both producer and consumer. Do not add fields or semantics to v1 that require v1 consumers to understand v2 behavior.

The CLI is explicitly a current-shape consumer until its separate migration is implemented. Core Contract v1 documentation does not imply CLI v1 envelope support, and this contract task does not modify CLI.
