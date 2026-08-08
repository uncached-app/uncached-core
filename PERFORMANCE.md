# Performance

## Pruning Lookup

Run the reproducible baseline/optimized comparison with:

```text
cargo bench --features bench --bench pruning
```

The Criterion group compares the previous linear `Vec<PathBuf>` lookup with
the production `HashSet<PathBuf>` plus `Path::ancestors()` lookup. Fixtures
are deterministic and generated outside timed closures. Workloads vary across
1,000, 100,000, and 1,000,000 entries; 1, 32, and 1,024 pruned targets;
depths 4, 12, and 32; and first-hit, last-hit, and miss cases. Throughput is
reported as entries per second.

The complete matrix is intentionally opt-in. On the development host, the
high-cardinality baseline last-hit cases make a full default Criterion run
impractical. A bounded smoke run of the 1,000-entry, one-target, depth-4,
first-hit case measured both implementations and emitted throughput. These
observations are host-specific and are not general performance claims:

```text
baseline:  [6.3839 Melem/s 7.7113 Melem/s 9.0452 Melem/s]
optimized: [1.9458 Melem/s 1.9813 Melem/s 2.0143 Melem/s]
```

The smoke run used `--warm-up-time 0.01 --measurement-time 0.01
--sample-size 10`. Criterion used its plotters backend because Gnuplot was
not installed. No incomplete full-matrix run is used for a performance claim.
