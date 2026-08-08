# Provider-Aware Scanning Design

## Goal

Make `uncached-core` own workspace discovery and category-aware trash cleanup for all currently active Desktop providers, while preserving CLI's NodeModules-only default.

## Scope

Active categories in this cycle:

- `NodeModules`
- `CargoTarget`
- `NextJs`
- `Turbo`
- `PyCache`
- `Gradle`

`DotNetObj` and `Docker` remain declared categories but have no detector in this cycle.

## Core Contract

`Category` becomes a shared serializable enum containing active and reserved categories. `Scanner` remains the provider extension point and continues to own name matching and context confirmation. `ProviderSet` owns configured scanners and can resolve a scanner by category.

`scan_workspace` collects every categorized traversal entry rather than hard-coding `NodeModules`. Each `ScanTarget` preserves the scanner category and filesystem identity. `ProviderSet::free()` remains NodeModules-only so CLI behavior is unchanged; Desktop constructs its configured provider set explicitly.

Core cleanup receives the configured `ProviderSet`. It validates the target's category using the matching provider, then applies shared root, symlink, identity, and safe-path checks before trash deletion. Missing provider or category/context mismatch returns `InvalidTarget`; no category silently falls back to NodeModules rules.

## Desktop Migration

Desktop scanners become implementations/adapters of the core scanner contract. Settings determine which active providers are registered. Scan service invokes core traversal/scan APIs once, converts core targets into UI targets, and retains existing asynchronous UI sizing and event behavior. Desktop-specific target IDs, formatting, and registry lifecycle remain in Desktop.

Trash cleanup reconstructs a core target from the registered UI target and delegates validation/deletion to core for all active categories. Permanent deletion remains Desktop-owned in this cycle.

## Error Handling

Provider resolution and contextual validation failures map to existing `CoreError::InvalidTarget`. Existing trash error mapping remains unchanged. Error enum redesign and Windows filesystem identity implementation are explicitly out of scope.

## Testing

Core tests will prove:

- scanning multiple provider categories in one traversal;
- preserving category and deterministic path ordering;
- pruning each accepted target;
- rejecting cleanup when category provider is absent or context is invalid;
- cleaning a non-NodeModules target through its provider.

Desktop tests will prove:

- settings produce the expected active provider set;
- core target conversion preserves category, path, and identity;
- NodeModules-only CLI behavior remains intact.

## Non-Goals

- Adding detection rules for `DotNetObj` or `Docker`.
- Windows `FILE_ID_INFO` identity support.
- Typed/serialized error redesign.
- Permanent deletion migration.
- Traversal parallelism changes.
