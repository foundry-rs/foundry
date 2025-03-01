use std::thread;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressFinish};

fn main() {
    let mut spinner: Option<ProgressBar> = None;

    for i in 0..3 {
        let new_spinner = ProgressBar::new_spinner()
            .with_message(format!("doing stuff {}", i))
            .with_finish(ProgressFinish::AndLeave);
        new_spinner.enable_steady_tick(Duration::from_millis(10));
        thread::sleep(Duration::from_millis(500));
        println!("\n\nreplace {}\n\n", i);
        if let Some(t) = spinner.replace(new_spinner) { t.finish() }
    }
}
