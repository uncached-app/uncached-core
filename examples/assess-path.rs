//! Prints the confidence assessment for each path given on the command line.
fn main() {
    for argument in std::env::args().skip(1) {
        let path = std::path::PathBuf::from(&argument);
        let input = uncached_core::collect_signals(&path, true);
        let confidence = uncached_core::assess(input);
        println!("{:?}  {}", confidence.level, argument);
        for signal in &confidence.signals {
            println!("      - {}", signal.reason());
        }
    }
}
