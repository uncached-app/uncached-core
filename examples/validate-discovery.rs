//! Checks that every catalogued location this machine actually has is one
//! cleanup would accept — the round trip the product makes, against a real
//! home directory rather than a fixture.
//!
//! ```sh
//! cargo run --example validate-discovery
//! ```

use uncached_core::discovery::{home_directory, probe_home, probe_temp, KnownLocationScanner};
use uncached_core::{validate_clean_target, LocationKind, ScanTarget};

fn main() {
    let home = home_directory().expect("a home directory");
    let providers = KnownLocationScanner::provider_set(home.clone());
    let mut found = probe_home(&home);
    found.extend(probe_temp(&home));

    for discovery in &found {
        if discovery.kind == LocationKind::ReportOnly {
            continue;
        }
        // Captured the way the product captures it: cleanup refuses a target
        // it cannot pin to an inode, which is what stops a directory swapped
        // under the app from being the one removed.
        // The real inode, captured the way `from_discovery` captures it.
        let identity = uncached_core::filesystem::file_identity(&discovery.path);
        let target = ScanTarget::new(discovery.path.clone(), discovery.category.clone(), 0)
            .with_identity(identity);
        match validate_clean_target(&discovery.root, &providers, &target) {
            Ok(_) => println!("  ok       {}", discovery.path.display()),
            Err(error) => println!("  REFUSED  {}  -> {:?}", discovery.path.display(), error),
        }
    }
}
