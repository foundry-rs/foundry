use std::io::{BufRead, BufReader};
use std::process;
use std::time::{Duration, Instant};

use indicatif::{HumanDuration, ProgressBar, ProgressStyle};

pub fn main() {
    let started = Instant::now();

    println!("Compiling package in release mode...");

    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(Duration::from_millis(200));
    pb.set_style(
        ProgressStyle::with_template("{spinner:.dim.bold} cargo: {wide_msg}")
            .unwrap()
            .tick_chars("/|\\- "),
    );

    let mut p = process::Command::new("cargo")
        .arg("build")
        .arg("--release")
        .stderr(process::Stdio::piped())
        .spawn()
        .unwrap();

    for line in BufReader::new(p.stderr.take().unwrap()).lines() {
        let line = line.unwrap();
        let stripped_line = line.trim();
        if !stripped_line.is_empty() {
            pb.set_message(stripped_line.to_owned());
        }
        pb.tick();
    }

    p.wait().unwrap();

    pb.finish_and_clear();

    println!("Done in {}", HumanDuration(started.elapsed()));
}
