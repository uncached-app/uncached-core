# Serializable Core Errors Task 1 Report

## Status

Implemented Task 1 and review fixes on core main at final report HEAD `4d9a816`.

## Commits

- `da7da97` (`feat(core): add serializable error DTO`)
- `84b8f13` (`fix(core): sanitize public error messages`)
- `4d9a816` (`docs: update serializable error report`)

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

The full core test run included 37 unit tests, 41 `node_modules` integration tests,
2 `providers` integration tests, and 0 doc tests.

## Review Fix

- Sanitized `CoreErrorInfo.message` to stable variant-level public text.
- Kept path and operation context in their structured fields.
- Preserved native source text in `CoreError` `Display` and `Error::source` behavior.
- Added regression assertions using unique private native detail and native source formatting.

## Updated TDD Evidence

RED command:

```text
cargo test error::tests
```

RED result: 2 tests failed because `public_info` still serialized native source text in `message`.

GREEN command:

```text
cargo test error::tests
```

GREEN result: 7 passed, 0 failed.

## Updated Verification

```text
cargo test error::tests       # 7 passed, 0 failed
cargo test                    # 37 unit + 41 node_modules integration + 2 providers integration + 0 doc = 80 passed, 0 failed
cargo fmt --all               # completed successfully
cargo fmt --all -- --check    # completed successfully
```

## Concern

None. Native source details remain available through `CoreError` for logs and debugging but do not enter serialized `CoreErrorInfo` JSON.
