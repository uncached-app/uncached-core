//! How much of a traversal is spent deciding what each directory is.
//!
//!     cargo run --release --example classify-cost -- <directory>
//!
//! Walks the directory with every built-in provider, then times `classify`
//! alone over the same names. The directory must be an acceptable workspace:
//! system roots such as `/usr` or `/root` are refused, as in a real scan.
//!
//! Warm-cache figures: on a cold disk the walk is slower and the share smaller.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use uncached_core::{traverse_workspace, ProviderSet, TraversalControl};

fn main() {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: classify-cost <directory>"),
    );
    let providers = ProviderSet::all();
    let cancelled = AtomicBool::new(false);
    let mut entries = Vec::new();

    let started = Instant::now();
    traverse_workspace(&root, &providers, &cancelled, |entry| {
        entries.push((entry.name.clone(), entry.path.clone()));
        if entry.category.is_some() {
            TraversalControl::Prune
        } else {
            TraversalControl::Continue
        }
    })
    .expect("the directory should be an acceptable workspace");
    let traversal = started.elapsed();

    // Repeated so the figure is stable; the walk above is a single pass.
    let passes = 20u32;
    let started = Instant::now();
    for _ in 0..passes {
        for (name, path) in &entries {
            std::hint::black_box(providers.classify(name, path));
        }
    }
    let classify = started.elapsed() / passes;

    println!("directories      : {}", entries.len());
    println!("providers        : {}", providers.len());
    println!("traversal        : {traversal:?}");
    println!(
        "classify alone   : {classify:?} ({:.1}% of the traversal)",
        classify.as_secs_f64() / traversal.as_secs_f64() * 100.0
    );
    if !entries.is_empty() {
        println!("per directory    : {:?}", classify / entries.len() as u32);
    }
}
