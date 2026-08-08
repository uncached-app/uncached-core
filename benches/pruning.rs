use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use uncached_core::scanner::PrunedPaths;
use uncached_core::{traverse_workspace, ProviderSet, TraversalControl};

const ENTRY_COUNTS: &[usize] = &[1_000, 100_000, 1_000_000];
const PRUNED_COUNTS: &[usize] = &[1, 32, 1_024];
const DEPTHS: &[usize] = &[4, 12, 32];
const CASES: &[&str] = &["first-ancestor-hit", "last-ancestor-hit", "miss"];

struct Fixture {
    entries: Vec<PathBuf>,
    baseline_pruned: Vec<PathBuf>,
    hybrid_pruned: PrunedPaths,
}

fn baseline_contains(pruned: &[PathBuf], path: &Path) -> bool {
    pruned.iter().any(|ancestor| path.starts_with(ancestor))
}

fn target_path(target: usize) -> PathBuf {
    PathBuf::from(format!("/fixture/pruned/target-{target}"))
}

fn lookup_path(case: &str, pruned_count: usize, depth: usize) -> PathBuf {
    let mut path = match case {
        "first-ancestor-hit" => target_path(0),
        "last-ancestor-hit" => target_path(pruned_count - 1),
        "miss" => PathBuf::from("/fixture/miss"),
        _ => unreachable!("case is defined by CASES"),
    };
    for component in 0..depth {
        path.push(format!("level-{component}"));
    }
    path
}

fn fixture(entry_count: usize, pruned_count: usize, depth: usize, case: &str) -> Fixture {
    let baseline_pruned: Vec<_> = (0..pruned_count).map(target_path).collect();
    let mut hybrid_pruned = PrunedPaths::default();
    for path in &baseline_pruned {
        hybrid_pruned.insert(path.clone());
    }
    let path = lookup_path(case, pruned_count, depth);
    let entries = vec![path; entry_count];
    Fixture {
        entries,
        baseline_pruned,
        hybrid_pruned,
    }
}

fn pruning_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("pruning_lookup");

    for &entry_count in ENTRY_COUNTS {
        for &pruned_count in PRUNED_COUNTS {
            for &depth in DEPTHS {
                for &case in CASES {
                    let fixture = fixture(entry_count, pruned_count, depth, case);
                    let id = format!("{entry_count}/{pruned_count}/{depth}/{case}");
                    group.throughput(Throughput::Elements(entry_count as u64));

                    group.bench_with_input(
                        BenchmarkId::new("baseline", &id),
                        &fixture,
                        |b, fixture| {
                            b.iter(|| {
                                let mut hits = 0usize;
                                for path in &fixture.entries {
                                    hits += black_box(baseline_contains(
                                        &fixture.baseline_pruned,
                                        path,
                                    )) as usize;
                                }
                                black_box(hits)
                            });
                        },
                    );
                    group.bench_with_input(
                        BenchmarkId::new("hybrid", &id),
                        &fixture,
                        |b, fixture| {
                            b.iter(|| {
                                let mut hits = 0usize;
                                for path in &fixture.entries {
                                    hits += black_box(fixture.hybrid_pruned.contains_ancestor(path))
                                        as usize;
                                }
                                black_box(hits)
                            });
                        },
                    );
                }
            }
        }
    }
    group.finish();
}

fn candidate_fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "uncached-core-pruning-candidates-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    for branch in 0..24 {
        let mut path = root.join("irrelevant");
        for level in 0..branch {
            path = path.join(format!("level-{level}"));
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("ignored.txt"), b"irrelevant").unwrap();
        }
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(format!("file-{branch}.txt")), b"irrelevant").unwrap();
    }

    for artifact in 0..12 {
        let path = root
            .join(format!("project-{artifact}"))
            .join("node_modules");
        fs::create_dir_all(&path).unwrap();
        for file in 0..256 {
            fs::write(
                path.join(format!("package-{file}.bin")),
                b"terminal artifact",
            )
            .unwrap();
        }
        fs::write(path.parent().unwrap().join("package.json"), b"{}").unwrap();
    }

    for target in 0..4 {
        let path = root
            .join("valid")
            .join(format!("project-{target}"))
            .join("node_modules");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.parent().unwrap().join("package.json"), b"{}").unwrap();
    }

    root
}

fn callback_only_discovery(c: &mut Criterion) {
    let root = candidate_fixture();
    let cancelled = AtomicBool::new(false);
    let mut expected_candidates = 0usize;
    traverse_workspace(&root, &ProviderSet::free(), &cancelled, |entry| {
        if entry.category.is_some() {
            expected_candidates += 1;
            TraversalControl::Prune
        } else {
            TraversalControl::Continue
        }
    })
    .unwrap();
    assert_eq!(expected_candidates, 16);

    let mut group = c.benchmark_group("candidate_discovery");
    group.bench_function("callback_only", |b| {
        b.iter(|| {
            let cancelled = AtomicBool::new(false);
            let mut candidates = 0usize;
            traverse_workspace(
                black_box(&root),
                &ProviderSet::free(),
                &cancelled,
                |entry| {
                    if entry.category.is_some() {
                        candidates += 1;
                        TraversalControl::Prune
                    } else {
                        TraversalControl::Continue
                    }
                },
            )
            .unwrap();
            black_box(candidates)
        });
    });
    group.finish();
    let _ = fs::remove_dir_all(root);
}

criterion_group!(benches, pruning_lookup, callback_only_discovery);
criterion_main!(benches);
