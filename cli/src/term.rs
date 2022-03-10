//! terminal utils

use atty::{self, Stream};
use ethers::solc::{report::Reporter, CompilerInput, Solc};
use once_cell::sync::Lazy;
use semver::Version;
use std::{
    io,
    io::prelude::*,
    path::Path,
    sync::{
        mpsc::{self, TryRecvError},
        Arc, Mutex,
    },
};

/// Some spinners
// https://github.com/gernest/wow/blob/master/spin/spinners.go
pub static SPINNERS: &[&[&str]] = &[
    &["⠃", "⠊", "⠒", "⠢", "⠆", "⠰", "⠔", "⠒", "⠑", "⠘"],
    &[" ", "⠁", "⠉", "⠙", "⠚", "⠖", "⠦", "⠤", "⠠"],
    &["┤", "┘", "┴", "└", "├", "┌", "┬", "┐"],
    &["▹▹▹▹▹", "▸▹▹▹▹", "▹▸▹▹▹", "▹▹▸▹▹", "▹▹▹▸▹", "▹▹▹▹▸"],
    &[" ", "▘", "▀", "▜", "█", "▟", "▄", "▖"],
];

static TERM_SETTINGS: Lazy<TermSettings> = Lazy::new(TermSettings::from_env);

/// Helper type to determine the current tty
pub struct TermSettings {
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
    idx: usize,
}

#[allow(unused)]
impl Spinner {
    pub fn new(msg: impl Into<String>) -> Self {
        Self::with_indicator(SPINNERS[0], msg)
    }

    pub fn with_indicator(indicator: &'static [&'static str], msg: impl Into<String>) -> Self {
        Spinner {
            indicator,
            no_progress: !TERM_SETTINGS.indicate_progress,
            message: msg.into(),
            idx: 0,
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
        if self.idx >= self.indicator.len() {
            self.idx = 0;
        }

        let s = format!(
            "\r\x1b[2K\x1b[1m[\x1b[32m{}\x1b[0;1m]\x1b[0m {}",
            self.indicator[self.idx], self.message
        );
        self.idx += 1;

        s
    }

    pub fn done(&self) {
        if self.no_progress {
            return
        }
        println!("\r\x1b[2K\x1b[1m[\x1b[32m+\x1b[0;1m]\x1b[0m {}", self.message);
        io::stdout().flush().unwrap();
    }

    pub fn finish(&mut self, msg: impl Into<String>) {
        self.message(msg);
        self.done();
    }

    pub fn message(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
    }

    pub fn clear_line(&self) {
        if self.no_progress {
            return
        }
        print!("\r\x33[2K\r");
        io::stdout().flush().unwrap();
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
        println!("\r\x1b[2K\x1b[1m[\x1b[31m-\x1b[0;1m]\x1b[0m {}", line);
    }
}

/// A spinner used as [`ethers::solc::report::Reporter`]
///
/// This reporter will prefix messages with a spinning cursor
pub struct SpinnerReporter {
    /// the timeout in ms
    sender: Arc<Mutex<mpsc::Sender<SpinnerMsg>>>,
}
impl SpinnerReporter {
    /// Spawns the [`Spinner`] on a new thread
    ///
    /// The spinner's message will be updated via the `ethers::solc::Reporter` events
    ///
    /// On drop the channel will disconnect and the thread will terminate
    pub fn spawn() -> Self {
        let (sender, rx) = mpsc::channel::<SpinnerMsg>();
        std::thread::spawn(move || {
            let mut spinner = Spinner::new("Compiling...");
            loop {
                spinner.tick();
                match rx.try_recv() {
                    Ok(msg) => {
                        match msg {
                            SpinnerMsg::Msg(msg) => {
                                spinner.message(msg);
                                // new line so past messages are not overwritten
                                println!();
                            }
                            SpinnerMsg::Shutdown(ack) => {
                                // end with a newline
                                println!();
                                let _ = ack.send(());
                                break
                            }
                        }
                    }
                    Err(TryRecvError::Disconnected) => break,
                    Err(TryRecvError::Empty) => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
        });
        SpinnerReporter { sender: Arc::new(Mutex::new(sender)) }
    }

    fn send_msg(&self, msg: impl Into<String>) {
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(SpinnerMsg::Msg(msg.into()));
        }
    }
}

enum SpinnerMsg {
    Msg(String),
    Shutdown(mpsc::Sender<()>),
}

impl Drop for SpinnerReporter {
    fn drop(&mut self) {
        if let Ok(sender) = self.sender.lock() {
            let (tx, rx) = mpsc::channel();
            if sender.send(SpinnerMsg::Shutdown(tx)).is_ok() {
                let _ = rx.recv();
            }
        }
    }
}

impl Reporter for SpinnerReporter {
    fn on_solc_spawn(&self, _solc: &Solc, version: &Version, input: &CompilerInput) {
        self.send_msg(format!(
            "Compiling {} files with {}.{}.{}",
            input.sources.len(),
            version.major,
            version.minor,
            version.patch
        ))
    }

    /// Invoked before a new [`Solc`] bin is installed
    fn on_solc_installation_start(&self, version: &Version) {
        self.send_msg(format!("installing solc version \"{}\"", version));
    }

    /// Invoked before a new [`Solc`] bin was successfully installed
    fn on_solc_installation_success(&self, version: &Version) {
        self.send_msg(format!("Successfully installed solc {}", version));
    }

    fn on_unresolved_import(&self, import: &Path) {
        self.send_msg(format!("Unable to resolve imported file: \"{}\"", import.display()));
    }
}

/// Calls `f` within the [`SpinnerReporter`] that displays a spinning cursor to display solc
/// progress
pub fn with_spinner_reporter<T>(f: impl FnOnce() -> T) -> T {
    ethers::solc::report::with_scoped(
        &ethers::solc::report::Report::new(SpinnerReporter::spawn()),
        f,
    )
}

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
