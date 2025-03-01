use indicatif::ProgressBar;

fn many_units_of_easy_work(n: u64, label: &str) {
    let pb = ProgressBar::new(n);

    let mut sum = 0;
    for i in 0..n {
        // Any quick computation, followed by an update to the progress bar.
        sum += 2 * i + 3;
        pb.inc(1);
    }
    pb.finish();

    println!("[{}] Sum ({}) calculated in {:?}", label, sum, pb.elapsed());
}

fn main() {
    const N: u64 = 1 << 20;

    // Perform a long sequence of many simple computations monitored by a
    // default progress bar.
    many_units_of_easy_work(N, "Default progress bar ");
}
