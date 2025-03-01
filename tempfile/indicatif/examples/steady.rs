use std::{
    thread::sleep,
    time::{Duration, Instant},
};

use indicatif::{ProgressBar, ProgressIterator, ProgressStyle};

fn main() {
    let iterations = 1000;
    // Set the array with all the blocksizes to test
    let blocksizes: [usize; 7] = [16, 64, 256, 1024, 4096, 16384, 65536];

    // Set the array with all the durations to save
    let mut elapsed: [Duration; 7] = [Duration::ZERO; 7];

    for (pos, blocksize) in blocksizes.iter().enumerate() {
        // Set up the style for the progressbar
        let sty = ProgressStyle::default_spinner()
            .tick_strings(&[
                "▹▹▹▹▹",
                "▸▹▹▹▹",
                "▹▸▹▹▹",
                "▹▹▸▹▹",
                "▹▹▹▸▹",
                "▹▹▹▹▸",
                "▪▪▪▪▪",
            ])
            .template("{prefix} {pos:>4}/{len:4} Iterations per second: {per_sec} {spinner} {msg}")
            .unwrap();

        // Set up the progress bar and apply the style
        let pb = ProgressBar::new(iterations);
        pb.set_style(sty);
        pb.enable_steady_tick(Duration::from_millis(120));
        pb.set_prefix(format!("Doing test with Blocksize {:5?}:", blocksize));

        // Iterate for the given number of iterations
        // for _ in (0..iterations) {
        for _ in (0..iterations).progress_with(pb) {
            // pb.inc(1);
            // Take a timestamp for timemeasurement later on
            let now = Instant::now();
            sleep(Duration::from_millis(1));
            // Save the elapsed time for later evaluation
            elapsed[pos] += now.elapsed();
        }

        // pb.finish_using_style();
    }
}
