# Serializable Core Errors Task 1 Report

## Status

Implemented Task 1 on core main at HEAD `9372b21`.

## Changes

- Added public `CoreErrorCode` with all current `CoreError` variants and stable `snake_case` `Serialize` output.
- Added public `CoreErrorInfo` DTO containing exactly `code`, `message`, `path`, and `operation`.
- Added borrowing `CoreError::public_info(&self) -> CoreErrorInfo` with exhaustive mapping over every current variant.
- Preserved existing `CoreError` variants, `Display`, `Error::source`, and native `std::io::Error` storage.
- Converted path-bearing fields with `to_string_lossy().into_owned()`.
- Re-exported `CoreErrorCode` and `CoreErrorInfo` from `uncached_core`.
- Added unit coverage for code serialization, exhaustive mapping, path and operation context, generic context absence, DTO field shape, source-field exclusion, and preserved display/source behavior.
- No CLI or Desktop files changed.

## TDD Evidence

Tests were added before production types and conversion code.

RED command:

```text
cargo test error::tests
```

RED result: compilation failed because `CoreErrorCode`, `CoreErrorInfo`, and `CoreError::public_info` were absent. This was the expected feature-missing failure.

GREEN command:

```text
cargo test error::tests
```

GREEN result: 7 passed, 0 failed.

## Verification

```text
cargo test error::tests  # 7 passed, 0 failed
cargo test              # 80 passed, 0 failed
cargo fmt --all         # completed successfully
cargo fmt --all -- --check  # final formatting check
```

The full core test run included 37 unit tests, 41 integration tests, and 2 doc tests.

## Concern

Secure `Display` messages intentionally include native source text because the approved contract requires `CoreErrorInfo.message` to equal existing `Display` output. Native sources are not represented as a separate serialized field; the DTO contains only the four approved fields and `CoreError` remains non-serializable.
