# Pruning Task 3 Report

## Status

Completed documentation and final verification on `main`. Existing pruning
commits: `78dfcb1` and `0fd3989`. This task adds only `PERFORMANCE.md` and
this report; no generated benchmark artifacts are included.

## Verification

Passed:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`: 42 unit tests, 41 `node_modules` tests, 2 provider tests, and 0 doc tests failed
- `cargo bench --features bench --bench pruning --no-run`

## Benchmark Evidence

The required benchmark command is:

```text
cargo bench --features bench --bench pruning
```

A fresh bounded smoke run completed successfully:

```text
cargo bench --features bench --bench pruning -- '1000/1/4/first-ancestor-hit' --warm-up-time 0.01 --measurement-time 0.01 --sample-size 10
```

Observed throughput ranges:

```text
baseline:  [6.3839 Melem/s 7.7113 Melem/s 9.0452 Melem/s]
optimized: [1.9458 Melem/s 1.9813 Melem/s 2.0143 Melem/s]
```

These are measurements from this host and smoke configuration only. The full
162-case matrix was not used for a claim because high-cardinality baseline
last-hit cases exceed the practical host budget. Criterion reported that
Gnuplot was unavailable and selected the plotters backend.

## Invariant Review

- Public scanner signatures remain unchanged; `PrunedPaths` is public only under the `bench` feature for the benchmark target.
- Provider matching, cancellation checks, callback controls, traversal ordering, symlink handling, and protected-directory handling are unchanged.
- Production pruning uses component-aware `Path::ancestors()` membership. The only string-prefix-style lookup is the intentional benchmark baseline using `Path::starts_with`, which preserves component semantics.
- Benchmark fixtures are constructed before `b.iter` closures.
- No `target/criterion` artifacts are tracked or intended for commit.

## Concerns

The optimized lookup is not faster for the recorded small first-hit smoke
case. The benchmark remains comparative and covers the high-cardinality cases
where the indexed lookup is intended to avoid linear scans; no broad speedup
claim is made from this limited run.

## Report Path

`.superpowers/sdd/pruning-task-3-report.md`

## Review Finding Follow-up

The review finding was valid: the all-`HashSet` implementation regressed the
1-target smoke case to 1.98 Melem/s versus 7.71 Melem/s for baseline. The
implementation was changed to a measured hybrid. Up to eight unique pruned
targets use the original linear `Vec<PathBuf>::starts_with` lookup; the ninth
target builds an index and uses `HashSet<PathBuf>` plus `path.ancestors()`.

Focused tests cover both representations and all component-boundary cases.
Current bounded smoke results, using `--warm-up-time 0.01 --measurement-time
0.01 --sample-size 10`:

```text
1000/1/4/first-ancestor-hit
baseline: [4.9018 Melem/s 5.0769 Melem/s 5.1974 Melem/s]
hybrid:   [6.5768 Melem/s 7.4185 Melem/s 8.6332 Melem/s]

1000/32/4/last-ancestor-hit
baseline: [86.716 Kelem/s 118.38 Kelem/s 167.31 Kelem/s]
hybrid:   [855.68 Kelem/s 910.86 Kelem/s 961.71 Kelem/s]
```

The small-workload regression is resolved in this smoke run. Criterion used
the plotters backend because Gnuplot is unavailable. The complete 162-case
matrix remains opt-in; high-cardinality baseline last-hit cases exceed this
host's practical bounded-run budget.
