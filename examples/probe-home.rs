//! Prints the catalogued locations that exist on this machine, with their size
//! and the level they would be reported at.
//!
//! The catalogue is a set of claims about where tools keep their caches. This
//! is how those claims get checked against a real home directory instead of
//! against memory.
//!
//! ```sh
//! cargo run --example probe-home
//! ```

use uncached_core::confidence::assess;
use uncached_core::discovery::{
    home_directory, known_locations, probe_home, probe_temp, signals_for,
};
use uncached_core::filesystem::directory_size_report;

fn main() {
    let Some(home) = home_directory() else {
        eprintln!("no home directory in the environment");
        std::process::exit(1);
    };

    let mut found = probe_home(&home);
    // Scratch outside the home directory, which no catalogue entry can reach.
    found.extend(probe_temp(&home));
    println!(
        "{home}: {} of {} catalogued locations present\n",
        found.len(),
        known_locations().len(),
        home = home.display()
    );

    let mut rows = found
        .iter()
        .map(|discovery| {
            let bytes = directory_size_report(&discovery.path)
                .map(|report| report.bytes)
                .unwrap_or(0);
            (bytes, discovery)
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|(bytes, _)| std::cmp::Reverse(*bytes));

    let mut total = 0u64;
    for (bytes, discovery) in &rows {
        total += bytes;
        let level = assess(signals_for(discovery)).level;
        println!(
            "{:>10}  {:<8?}  {:<32}  {}",
            human(*bytes),
            level,
            discovery.label,
            discovery.path.display()
        );
    }
    println!("\n{:>10}  total", human(total));
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
