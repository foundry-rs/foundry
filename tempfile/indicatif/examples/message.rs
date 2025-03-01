use std::{thread, time::Duration};

use indicatif::ProgressBar;

fn main() {
    let pb = ProgressBar::new(100).with_message("Frobbing the widget");
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(30));
        pb.inc(1);
    }
}
