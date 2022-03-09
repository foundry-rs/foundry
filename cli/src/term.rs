//! terminal utils

use atty::{self, Stream};
use ethers::solc::report::Reporter;
use once_cell::sync::Lazy;
use std::{io, io::prelude::*};

/// Some spinners
// https://github.com/gernest/wow/blob/master/spin/spinners.go
pub static SPINNERS: &[&[&str]] = &[
    &["⠃", "⠊", "⠒", "⠢", "⠆", "⠰", "⠔", "⠒", "⠑", "⠘"],
    &[" ", "⠁", "⠉", "⠙", "⠚", "⠖", "⠦", "⠤", "⠠"],
    &["┤", "┘", "┴", "└", "├", "┌", "┬", "┐"],
    &["▹▹▹▹▹", "▸▹▹▹▹", "▹▸▹▹▹", "▹▹▸▹▹", "▹▹▹▸▹", "▹▹▹▹▸"],
    &[" ", "▘", "▀", "▜", "█", "▟", "▄", "▖"],
];

static TERM_SETTINGS: Lazy<TermSettings> = Lazy::new(|| TermSettings::from_env());

/// Helper type to determine the current tty
pub struct TermSettings {
    // colors: bool,
    indicate_progress: bool,
}

impl TermSettings {
    pub fn from_env() -> TermSettings {
        if atty::is(Stream::Stdout) {
            TermSettings { indicate_progress: true }
        } else {
            TermSettings { indicate_progress: false }
        }
    }
}

pub struct Spinner {
    indicator: &'static [&'static str],
    no_progress: bool,
    message: String,
    i: usize,
}

impl Spinner {
    pub fn new(msg: impl Into<String>) -> Self {
        Self::with_indicator(SPINNERS[0], msg)
    }

    pub fn with_indicator(indicator: &'static [&'static str], msg: impl Into<String>) -> Self {
        Spinner {
            indicator,
            no_progress: !TERM_SETTINGS.indicate_progress,
            message: msg.into(),
            i: 0,
        }
    }

    pub fn tick(&mut self) {
        if self.no_progress {
            return
        }
        print!("{}", self.tick_bytes());
        io::stdout().flush().unwrap();
    }

    fn tick_bytes(&mut self) -> String {
        if self.i >= self.indicator.len() {
            self.i = 0;
        }

        let s = format!(
            "\r\x1b[2K\x1b[1m[\x1b[32m{}\x1b[0;1m]\x1b[0m {}...",
            self.indicator[self.i], self.message
        );
        self.i += 1;

        s
    }

    pub fn done(&self) {
        if self.no_progress {
            return
        }
        println!("\r\x1b[2K\x1b[1m[\x1b[32m{}\x1b[0;1m]\x1b[0m {}", '+', self.message);
        io::stdout().flush().unwrap();
    }

    pub fn finish(&mut self, msg: impl Into<String>) {
        self.message(msg);
        self.done();
    }

    pub fn message(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
    }

    pub fn clear(&self) {
        if self.no_progress {
            return
        }
        print!("\r\x1b[2K");
        io::stdout().flush().unwrap();
    }

    pub fn fail(&mut self, err: &str) {
        self.error(err);
        self.clear();
    }

    pub fn error(&mut self, line: &str) {
        if self.no_progress {
            return
        }
        println!("\r\x1b[2K\x1b[1m[\x1b[31m{}\x1b[0;1m]\x1b[0m {}", '-', line);
    }
}

/// A spinner used as [`ethers::solc::report::Reporter`]
pub struct SpinnerReporter {}

impl Reporter for SpinnerReporter {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn can_spin() {
        let mut s = Spinner::new("Compiling".to_string());
        let ticks = 50;
        for _ in 0..ticks {
            std::thread::sleep(std::time::Duration::from_millis(100));
            s.tick();
        }

        s.finish("Done".to_string());
    }
}
