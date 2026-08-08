# Pruning Task 2 Report

## Status

Implemented Criterion 0.7 baseline and optimized pruning benchmarks. Working tree was on `main`; Task 1 was already present in commit `78dfcb1`.

## Changes

- Added `bench = []` feature, Criterion `0.7` with `html_reports`, and non-harness `pruning` target.
- Added deterministic benchmark fixtures for 1,000, 100,000, and 1,000,000 entries; 1, 32, and 1,024 pruned targets; depths 4, 12, and 32; and first-hit, last-hit, and miss cases.
- Added baseline `Vec<PathBuf>` plus `Path::starts_with` lookup and optimized production `PrunedPaths` lookup.
- Added benchmark-only visibility for `PrunedPaths`; disabled-feature builds keep the type private and scanner public API unchanged.
- Updated `Cargo.lock` for Criterion dependencies.

## Verification

Passed:

- `cargo bench --features bench --bench pruning --no-run`
- `cargo bench --features bench --bench pruning -- '1000/1/4/first-ancestor-hit'`
- `cargo test` (40 unit tests, 41 node_modules tests, 2 provider tests, 2 doc tests)
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`

The requested `baseline/1000` command started successfully and emitted Criterion throughput, but its filter includes every 1,000-entry baseline combination and exceeded the 120-second command limit.

## Benchmark Output

The bounded smoke run measured both implementations. Representative output:

```text
baseline/1000/1/4/first-ancestor-hit
thrpt: [8.9719 Melem/s 9.0196 Melem/s 9.0538 Melem/s]

optimized/1000/1/4/first-ancestor-hit
thrpt: [1.8454 Melem/s 1.8553 Melem/s 1.8635 Melem/s]
```

A reduced-time all-dimensions run was started with `--warm-up-time 0.01 --measurement-time 0.01 --sample-size 10`. It reached 1,000,000-entry cases but timed out after 15 minutes because high-cardinality baseline last-hit cases require seconds per sample. No performance claim is inferred from the incomplete run. Criterion also reported `Gnuplot not found`; it used the plotters backend.

## Concerns

- Default Criterion settings for the complete 162-case baseline/optimized matrix are impractical on this machine, especially 1,024-target last-hit cases. The benchmark target remains complete and opt-in.
- Smoke numbers are machine-specific observations only, not repository performance claims.
- No traversal, provider, cancellation, or filesystem behavior was changed.
