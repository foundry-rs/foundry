use std::sync::Arc;
use std::thread;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

fn main() {
    let m = Arc::new(MultiProgress::new());
    let sty = ProgressStyle::with_template("{bar:40.green/yellow} {pos:>7}/{len:7}").unwrap();

    let pb = m.add(ProgressBar::new(5));
    pb.set_style(sty.clone());

    // make sure we show up at all.  otherwise no rendering
    // event.
    pb.tick();
    for _ in 0..5 {
        let pb2 = m.add(ProgressBar::new(128));
        pb2.set_style(sty.clone());
        for _ in 0..128 {
            thread::sleep(Duration::from_millis(5));
            pb2.inc(1);
        }
        pb2.finish();
        pb.inc(1);
    }
    pb.finish_with_message("done");
}
