//! terminal utils
use foundry_compilers::{
    artifacts::remappings::Remapping,
    report::{self, Reporter},
};
use itertools::Itertools;
use semver::Version;
use std::{
    fmt, io,
    io::{IsTerminal, prelude::*},
    path::{Path, PathBuf},
    sync::{
        LazyLock,
        mpsc::{self, TryRecvError},
    },
    thread,
    time::Duration,
};
use yansi::Paint;

use crate::shell;

/// Some spinners
// https://github.com/gernest/wow/blob/master/spin/spinners.go
pub static SPINNERS: &[&[&str]] = &[
    &["⠃", "⠊", "⠒", "⠢", "⠆", "⠰", "⠔", "⠒", "⠑", "⠘"],
    &[" ", "⠁", "⠉", "⠙", "⠚", "⠖", "⠦", "⠤", "⠠"],
    &["┤", "┘", "┴", "└", "├", "┌", "┬", "┐"],
    &["▹▹▹▹▹", "▸▹▹▹▹", "▹▸▹▹▹", "▹▹▸▹▹", "▹▹▹▸▹", "▹▹▹▹▸"],
    &[" ", "▘", "▀", "▜", "█", "▟", "▄", "▖"],
];

static TERM_SETTINGS: LazyLock<TermSettings> = LazyLock::new(TermSettings::from_env);

/// Helper type to determine the current tty
pub struct TermSettings {
    indicate_progress: bool,
}

impl TermSettings {
    /// Returns a new [`TermSettings`], configured from the current environment.
    pub fn from_env() -> Self {
        Self { indicate_progress: std::io::stdout().is_terminal() }
    }
}

#[expect(missing_docs)]
pub struct Spinner {
    indicator: &'static [&'static str],
    no_progress: bool,
    message: String,
    idx: usize,
}

#[expect(missing_docs)]
impl Spinner {
    pub fn new(msg: impl Into<String>) -> Self {
        Self::with_indicator(SPINNERS[0], msg)
    }

    pub fn with_indicator(indicator: &'static [&'static str], msg: impl Into<String>) -> Self {
        Self {
            indicator,
            no_progress: !TERM_SETTINGS.indicate_progress,
            message: msg.into(),
            idx: 0,
        }
    }

    pub fn tick(&mut self) {
        if self.no_progress {
            return;
        }

        let indicator = self.indicator[self.idx % self.indicator.len()].green();
        let indicator = Paint::new(format!("[{indicator}]")).bold();
        let _ = sh_print!("\r\x1B[2K\r{indicator} {}", self.message);
        io::stdout().flush().unwrap();

        self.idx = self.idx.wrapping_add(1);
    }

    pub fn message(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
    }
}

/// A spinner used as [`report::Reporter`]
///
/// This reporter will prefix messages with a spinning cursor
#[derive(Debug)]
#[must_use = "Terminates the spinner on drop"]
pub struct SpinnerReporter {
    /// The sender to the spinner thread.
    sender: mpsc::Sender<SpinnerMsg>,
    /// The project root path for trimming file paths in verbose output.
    project_root: Option<PathBuf>,
}

impl SpinnerReporter {
    /// Spawns the [`Spinner`] on a new thread
    ///
    /// The spinner's message will be updated via the `reporter` events
    ///
    /// On drop the channel will disconnect and the thread will terminate
    pub fn spawn(project_root: Option<PathBuf>) -> Self {
        let (sender, rx) = mpsc::channel::<SpinnerMsg>();

        std::thread::Builder::new()
            .name("spinner".into())
            .spawn(move || {
                let mut spinner = Spinner::new("Compiling...");
                loop {
                    spinner.tick();
                    match rx.try_recv() {
                        Ok(SpinnerMsg::Msg(msg)) => {
                            spinner.message(msg);
                            // new line so past messages are not overwritten
                            let _ = sh_println!();
                        }
                        Ok(SpinnerMsg::Shutdown(ack)) => {
                            // end with a newline
                            let _ = sh_println!();
                            let _ = ack.send(());
                            break;
                        }
                        Err(TryRecvError::Disconnected) => break,
                        Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(100)),
                    }
                }
            })
            .expect("failed to spawn thread");

        Self { sender, project_root }
    }

    fn send_msg(&self, msg: impl Into<String>) {
        let _ = self.sender.send(SpinnerMsg::Msg(msg.into()));
    }
}

enum SpinnerMsg {
    Msg(String),
    Shutdown(mpsc::Sender<()>),
}

impl Drop for SpinnerReporter {
    fn drop(&mut self) {
        let (tx, rx) = mpsc::channel();
        if self.sender.send(SpinnerMsg::Shutdown(tx)).is_ok() {
            let _ = rx.recv();
        }
    }
}

impl Reporter for SpinnerReporter {
    fn on_compiler_spawn(&self, compiler_name: &str, version: &Version, dirty_files: &[PathBuf]) {
        // Verbose message with dirty files displays first to avoid being overlapped
        // by the spinner in .tick() which prints repeatedly over the same line.
        if shell::verbosity() >= 5 {
            self.send_msg(format!(
                "Files to compile:\n{}",
                dirty_files
                    .iter()
                    .map(|path| {
                        let trimmed_path = if let Some(project_root) = &self.project_root {
                            path.strip_prefix(project_root).unwrap_or(path)
                        } else {
                            path
                        };
                        format!("- {}", trimmed_path.display())
                    })
                    .sorted()
                    .format("\n")
            ));
        }

        self.send_msg(format!(
            "Compiling {} files with {} {}.{}.{}",
            dirty_files.len(),
            compiler_name,
            version.major,
            version.minor,
            version.patch
        ));
    }

    fn on_compiler_success(&self, compiler_name: &str, version: &Version, duration: &Duration) {
        self.send_msg(format!(
            "{} {}.{}.{} finished in {duration:.2?}",
            compiler_name, version.major, version.minor, version.patch
        ));
    }

    fn on_solc_installation_start(&self, version: &Version) {
        self.send_msg(format!("Installing Solc version {version}"));
    }

    fn on_solc_installation_success(&self, version: &Version) {
        self.send_msg(format!("Successfully installed Solc {version}"));
    }

    fn on_solc_installation_error(&self, version: &Version, error: &str) {
        self.send_msg(format!("Failed to install Solc {version}: {error}").red().to_string());
    }

    fn on_unresolved_imports(&self, imports: &[(&Path, &Path)], remappings: &[Remapping]) {
        self.send_msg(report::format_unresolved_imports(imports, remappings));
    }
}

/// A pipe-safe [`Reporter`] for non-terminal stdout.
///
/// This is a drop-in replacement for [`BasicStdoutReporter`] that gracefully
/// handles `BrokenPipe` errors instead of panicking via `println!()`.  Some
/// widely-deployed `tee` implementations (notably uutils-coreutils ≤ 0.8) close
/// the pipe early when there is a pause between writes (e.g. during Solc
/// compilation), so we must treat `BrokenPipe` as non-fatal.
///
/// **Maintenance note:** the format strings here intentionally mirror those in
/// [`BasicStdoutReporter`].  If the upstream reporter changes its output,
/// update this implementation to match.
///
/// [`BasicStdoutReporter`]: foundry_compilers::report::BasicStdoutReporter
#[derive(Clone, Copy, Debug, Default)]
pub struct SafeStdoutReporter;

impl Reporter for SafeStdoutReporter {
    fn on_compiler_spawn(&self, compiler_name: &str, version: &Version, dirty_files: &[PathBuf]) {
        write_report_line(
            io::stdout().lock(),
            format_args!(
                "Compiling {} files with {} {}.{}.{}",
                dirty_files.len(),
                compiler_name,
                version.major,
                version.minor,
                version.patch
            ),
        );
    }

    fn on_compiler_success(&self, compiler_name: &str, version: &Version, duration: &Duration) {
        write_report_line(
            io::stdout().lock(),
            format_args!(
                "{} {}.{}.{} finished in {duration:.2?}",
                compiler_name, version.major, version.minor, version.patch
            ),
        );
    }

    fn on_solc_installation_start(&self, version: &Version) {
        write_report_line(
            io::stdout().lock(),
            format_args!("installing solc version \"{version}\""),
        );
    }

    fn on_solc_installation_success(&self, version: &Version) {
        write_report_line(
            io::stdout().lock(),
            format_args!("Successfully installed solc {version}"),
        );
    }

    fn on_solc_installation_error(&self, version: &Version, error: &str) {
        write_report_line(
            io::stderr().lock(),
            format_args!("Failed to install solc {version}: {error}"),
        );
    }

    fn on_unresolved_imports(&self, imports: &[(&Path, &Path)], remappings: &[Remapping]) {
        if imports.is_empty() {
            return;
        }

        write_report_line(
            io::stdout().lock(),
            format_args!("{}", report::format_unresolved_imports(imports, remappings)),
        );
    }
}

/// Write a single line to `writer`, silently discarding `BrokenPipe` errors.
///
/// Any other I/O error is logged at `trace` level rather than propagated,
/// because [`Reporter`] callbacks have no way to signal failure.
fn write_report_line(mut writer: impl io::Write, args: fmt::Arguments<'_>) {
    if let Err(err) = writeln!(writer, "{args}")
        && err.kind() != io::ErrorKind::BrokenPipe
    {
        trace!(target: "foundry_common::term", ?err, "failed to write compiler report output");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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
        let r = SpinnerReporter::spawn(None);
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

    #[test]
    fn write_report_line_ignores_broken_pipe() {
        struct BrokenPipeWriter;

        impl io::Write for BrokenPipeWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        write_report_line(BrokenPipeWriter, format_args!("hello"));
    }

    #[test]
    fn write_report_line_writes_newline_terminated_output() {
        #[derive(Clone, Default)]
        struct BufferWriter(Arc<Mutex<Vec<u8>>>);

        impl io::Write for BufferWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let writer = BufferWriter::default();
        let buffer = writer.0.clone();
        write_report_line(writer, format_args!("hello"));

        assert_eq!(String::from_utf8(buffer.lock().unwrap().clone()).unwrap(), "hello\n");
    }
}
