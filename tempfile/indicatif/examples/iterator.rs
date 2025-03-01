use std::thread;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressIterator, ProgressStyle};

fn main() {
    // Default styling, attempt to use Iterator::size_hint to count input size
    for _ in (0..1000).progress() {
        // ...
        thread::sleep(Duration::from_millis(5));
    }

    // Provide explicit number of elements in iterator
    for _ in (0..1000).progress_count(1000) {
        // ...
        thread::sleep(Duration::from_millis(5));
    }

    // Provide a custom bar style
    let pb = ProgressBar::new(1000);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] ({pos}/{len}, ETA {eta})",
        )
        .unwrap(),
    );
    for _ in (0..1000).progress_with(pb) {
        // ...
        thread::sleep(Duration::from_millis(5));
    }
}
