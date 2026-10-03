# Performance

## Pruning Lookup

Run the reproducible baseline/optimized comparison with:

```text
cargo bench --features bench --bench pruning
```

The Criterion group compares the previous linear `Vec<PathBuf>` lookup with
the production hybrid lookup. Sets up to eight unique targets use linear
`Path::starts_with`; larger sets use `HashSet<PathBuf>` plus
`Path::ancestors()`. Fixtures are deterministic and generated outside timed
closures. The default complete matrix uses 1,000 and 100,000 entries; 1, 32,
and 1,024 pruned targets; depths 4, 12, and 32; and first-hit, last-hit, and
miss cases. Throughput is reported as entries per second.

The 1,000,000-entry stress workload remains available without making normal
verification impractical:

```text
UNCACHED_RUN_STRESS_BENCH=1 cargo bench --features bench --bench pruning
```

The stress matrix is intentionally opt-in. On the development host, the
high-cardinality baseline last-hit cases make it impractical as a default run.
The practical matrix is complete and suitable for normal verification. A bounded smoke run on the current hybrid implementation measured
the following cases. These observations are host-specific and are not general
performance claims:

```text
1000/1/4/first-ancestor-hit
baseline: [4.9018 Melem/s 5.0769 Melem/s 5.1974 Melem/s]
hybrid:   [6.5768 Melem/s 7.4185 Melem/s 8.6332 Melem/s]

1000/32/4/last-ancestor-hit
baseline: [86.716 Kelem/s 118.38 Kelem/s 167.31 Kelem/s]
hybrid:   [855.68 Kelem/s 910.86 Kelem/s 961.71 Kelem/s]
```

The smoke run used `--warm-up-time 0.01 --measurement-time 0.01
--sample-size 10`. Criterion used its plotters backend because Gnuplot was
not installed. No incomplete full-matrix run is used for a performance claim.

## Candidate-Directed Discovery

The pruning benchmark also exercises the production traversal with a deterministic
filesystem fixture. It creates 24 irrelevant branches, 12 project `node_modules`
directories containing 256 terminal files each, and 4 valid project candidates.
The traversal confirmed 16 candidates and returned `TraversalControl::Prune` for
each one. Terminal artifact files were not observed by the callback, confirming
that discovery does not descend into confirmed artifact directories.

A filtered run used:

```text
cargo bench --bench pruning --features bench candidate_discovery -- --warm-up-time 0.1 --measurement-time 0.5 --sample-size 10
```

It measured `candidate_discovery/callback_only` at
`[14.361 ms 14.596 ms 14.983 ms]` on this host. Criterion reported that the
short smoke configuration needed about 883 ms to collect all 10 samples; this
result is an observation, not a cross-host performance target. The default
practical matrix is complete. The opt-in stress matrix is excluded from normal
verification because high-cardinality cases exceed the available window.

Sizing is a Desktop lifecycle concern rather than part of this core benchmark:
Desktop caps sizing workers at 8, uses a bounded queue, and reports completion
only after all sizing jobs settle. A sizing error settles visibly on its target;
it does not silently become a successful size or permit cleanup.

## Secure Open

Cleanup on Windows opens the target before mutating anything. What gets opened
depends on the cleanup mode, and that dominates the wall-clock cost of a cleanup.

Permanent cleanup deletes through retained handles, so it must open every entry
in the subtree. Trash cleanup hands a path to the Shell and releases every handle
before that call, so descendant handles could never guard the operation; it opens
only the target and its ancestors.

Measured on `x86_64-pc-windows-msvc` with a synthetic target of 10,000 files in
400 directories, timing only the open phase. Host-specific observations, not
general performance claims:

```text
permanent, sequential collection: 497.7 ms
permanent, parallel collection:   124.9 ms
trash:                              0.15 ms
```

Parallel collection expands one directory level at a time across the shared
worker pool. Entries are appended in frontier order rather than completion order,
so the collection stays deterministic and a parent always precedes its
descendants — the invariant permanent deletion relies on when it drains the
collection in reverse.

## Provider Classification

`ProviderSet::classify` asks each registered provider, in order, whether it
matches a directory name, and only for a match does it look at the filesystem
(a marker file, a `.gitignore`). With the full built-in set of 29 providers the
name test therefore runs 29 times for every directory that matches nothing,
which is nearly all of them.

Measured with the example that ships for the purpose:

```text
cargo run --release --example classify-cost -- <directory>
```

On the development host, over a 1,678-directory copy of the Cargo registry
sources, and over a 71-directory checkout, warm cache:

```text
classify alone: about 640-760 ns per directory
share of the walk: about 5-9 per cent
```

Those figures are host-specific, vary by a few per cent between runs, and are
not general performance claims. The walk itself costs about 8 microseconds per
directory here, and on a cold disk it costs far more, so the share only gets
smaller.

That is why classification is not indexed by name. An index would need the
public `Scanner` trait to say which names a provider claims, and would save at
most the few milliseconds a 100,000-directory scan spends on name tests,
against seconds of directory reads. If a future provider makes `matches` or
`confirm_context` expensive, measure again with the example above before
changing anything.
