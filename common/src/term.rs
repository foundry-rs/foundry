//! terminal utils
use ethers_solc::{
    remappings::Remapping,
    report::{self, BasicStdoutReporter, Reporter, SolcCompilerIoReporter},
    CompilerInput, CompilerOutput, Solc,
};
use once_cell::sync::Lazy;
use semver::Version;
use std::{
    io,
    io::prelude::*,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, TryRecvError},
        Arc, Mutex,
    },
    time::Duration,
};
use yansi::Paint;

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
    #[allow(missing_docs)]
    pub fn from_env() -> TermSettings {
        if is_terminal::is_terminal(std::io::stdout()) {
            TermSettings { indicate_progress: true }
        } else {
            TermSettings { indicate_progress: false }
        }
    }
}

#[allow(missing_docs)]
pub struct Spinner {
    indicator: &'static [&'static str],
    no_progress: bool,
    message: String,
    idx: usize,
}

#[allow(unused)]
#[allow(missing_docs)]
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

        print!(
            "\r\x33[2K\r{} {}",
            Paint::new(format!("[{}]", Paint::green(self.indicator[self.idx]))).bold(),
            self.message
        );
        io::stdout().flush().unwrap();

        self.idx += 1;
        if self.idx >= self.indicator.len() {
            self.idx = 0;
        }
    }

    pub fn message(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
    }
}

/// A spinner used as [`report::Reporter`]
///
/// This reporter will prefix messages with a spinning cursor
#[derive(Debug)]
pub struct SpinnerReporter {
    /// the timeout in ms
    sender: Arc<Mutex<mpsc::Sender<SpinnerMsg>>>,
    /// A reporter that logs solc compiler input and output to separate files if configured via env
    /// var
    solc_io_report: SolcCompilerIoReporter,
}
impl SpinnerReporter {
    /// Spawns the [`Spinner`] on a new thread
    ///
    /// The spinner's message will be updated via the `reporter` events
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
        SpinnerReporter {
            sender: Arc::new(Mutex::new(sender)),
            solc_io_report: SolcCompilerIoReporter::from_default_env(),
        }
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
    fn on_solc_spawn(
        &self,
        _solc: &Solc,
        version: &Version,
        input: &CompilerInput,
        dirty_files: &[PathBuf],
    ) {
        self.send_msg(format!(
            "Compiling {} files with {}.{}.{}",
            dirty_files.len(),
            version.major,
            version.minor,
            version.patch
        ));
        self.solc_io_report.log_compiler_input(input, version);
    }

    fn on_solc_success(
        &self,
        _solc: &Solc,
        version: &Version,
        output: &CompilerOutput,
        duration: &Duration,
    ) {
        self.solc_io_report.log_compiler_output(output, version);
        self.send_msg(format!(
            "Solc {}.{}.{} finished in {duration:.2?}",
            version.major, version.minor, version.patch
        ));
    }

    /// Invoked before a new [`Solc`] bin is installed
    fn on_solc_installation_start(&self, version: &Version) {
        self.send_msg(format!("Installing solc version {version}"));
    }

    /// Invoked before a new [`Solc`] bin was successfully installed
    fn on_solc_installation_success(&self, version: &Version) {
        self.send_msg(format!("Successfully installed solc {version}"));
    }

    fn on_solc_installation_error(&self, version: &Version, error: &str) {
        self.send_msg(Paint::red(format!("Failed to install solc {version}: {error}")).to_string());
    }

    fn on_unresolved_imports(&self, imports: &[(&Path, &Path)], remappings: &[Remapping]) {
        self.send_msg(report::format_unresolved_imports(imports, remappings));
    }
}

/// If the output medium is terminal, this calls `f` within the [`SpinnerReporter`] that displays a
/// spinning cursor to display solc progress.
///
/// If no terminal is available this falls back to common `println!` in [`BasicStdoutReporter`].
pub fn with_spinner_reporter<T>(f: impl FnOnce() -> T) -> T {
    let reporter = if TERM_SETTINGS.indicate_progress {
        report::Report::new(SpinnerReporter::spawn())
    } else {
        report::Report::new(BasicStdoutReporter::default())
    };
    report::with_scoped(&reporter, f)
}

#[macro_export]
/// Displays warnings on the cli
macro_rules! cli_warn {
    ($($arg:tt)*) => {
        eprintln!(
            "{}{} {}",
            yansi::Paint::yellow("warning").bold(),
            yansi::Paint::new(":").bold(),
            format_args!($($arg)*)
        )
    }
}

pub use cli_warn;

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
    }

    #[test]
    fn can_format_properly() {
        let r = SpinnerReporter::spawn();
        let remappings: Vec<Remapping> = vec![
            "library/=library/src/".parse().unwrap(),
            "weird-erc20/=lib/weird-erc20/src/".parse().unwrap(),
            "ds-test/=lib/ds-test/src/".parse().unwrap(),
            "openzeppelin-contracts/=lib/openzeppelin-contracts/contracts/".parse().unwrap(),
        ];
        let unresolved = vec![(Path::new("./src/Import.sol"), Path::new("src/File.col"))];
        r.on_unresolved_imports(&unresolved, &remappings);
        // formats:
        // [⠒] Unable to resolve imports:
        //       "./src/Import.sol" in "src/File.col"
        // with remappings:
        //       library/=library/src/
        //       weird-erc20/=lib/weird-erc20/src/
        //       ds-test/=lib/ds-test/src/
        //       openzeppelin-contracts/=lib/openzeppelin-contracts/contracts/
    }
}
