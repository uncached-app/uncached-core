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
closures. Workloads vary across 1,000, 100,000, and 1,000,000 entries; 1, 32,
and 1,024 pruned targets; depths 4, 12, and 32; and first-hit, last-hit, and
miss cases. Throughput is reported as entries per second.

The complete matrix is intentionally opt-in. On the development host, the
high-cardinality baseline last-hit cases make a full default Criterion run
impractical. A bounded smoke run on the current hybrid implementation measured
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
