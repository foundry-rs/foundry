// TODO remove later
#![allow(dead_code)]
mod args;

pub use args::Args;
mod cmd;
mod complete;
mod config;
mod input;
mod paths;
mod session;
mod term;
pub use crate::config::Config;

use self::complete::SolReplCompleter;
use crate::{
    cmd::Cmd,
    input::{Command, Input},
    session::Session,
    term::Prompt,
};
use eyre::WrapErr;
use log::debug;
use rustyline::{error::ReadlineError, Editor};
use solang::parser::pt::SourceUnit;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

/// Solidity shell
#[derive(Debug)]
pub struct Shell {
    /// The line editor that provides user input
    pub(crate) rl: Editor<SolReplCompleter>,
    /// CTRL-C hook
    pub(crate) signal_register: Arc<SignalRegister>,
    /// Prompt to use when starting a new line
    pub(crate) prompt: Prompt,
    /// Internal context aware session
    pub(crate) session: Session,
    /// General config vals
    pub(crate) config: Config,
    /// internal CTRL-C counter
    ctrl_c_counter: u8,
}

impl Shell {
    pub fn new(args: Args, config: Config) -> eyre::Result<Self> {
        let rl = SolReplCompleter::default().into_editor();

        let mut session = Session::default();
        session.libs.extend(args.libs);

        let prompt = if let Some(workspace) = args.workspace {
            term::info(format!("loading workspace `{}`...", workspace.display()));
            let name = session.add_project(workspace)?;
            session.compile_all()?;
            Prompt::new(name)
        } else {
            Default::default()
        };

        Ok(Self {
            rl,
            signal_register: Arc::new(Default::default()),
            prompt,
            session,
            config,
            ctrl_c_counter: 0,
        })
    }

    /// Returns the current configured workspace if any
    pub fn workspace(&self) -> Option<&String> {
        self.prompt.project.as_ref()
    }

    pub fn load_history(&mut self) -> eyre::Result<()> {
        Ok(self.rl.load_history(&paths::history_path()?)?)
    }

    pub fn save_history(&mut self) -> eyre::Result<()> {
        Ok(self.rl.save_history(&paths::history_path()?)?)
    }

    /// Reads the next user input line, converts that into an Input
    pub fn readline(&mut self) -> Option<Input> {
        let readline = self.rl.readline(&self.prompt.to_string());

        if readline.is_ok() {
            self.ctrl_c_counter = 0;
        }

        match readline {
            Ok(line) => Input::read_line(line, self),
            Err(ReadlineError::Interrupted) => {
                // ^C
                self.ctrl_c_counter += 1;

                if self.ctrl_c_counter > 1 {
                    Some(Input::Command(Command::Interrupt, vec![]))
                } else {
                    None
                }
            }
            Err(ReadlineError::Eof) => {
                // ^D
                None
            }
            Err(err) => {
                println!("Error: {:?}", err);
                Some(Input::Command(Command::Interrupt, vec![]))
            }
        }
    }

    /// Registers the signal handler to terminate the program once we received a CTRL-C signal
    pub fn set_signal_handler(&self) -> eyre::Result<()> {
        let ctr = self.signal_register.clone();
        ctrlc::set_handler(move || {
            let prev = ctr.add_ctrlc();
            eprintln!("prev {}", prev);
            if prev == 1 {
                ::std::process::exit(0);
            }
        })?;

        Ok(())
    }

    /// Executes a native command
    fn run_cmd(&mut self, cmd: Command, args: Vec<String>) -> eyre::Result<bool> {
        match cmd {
            Command::Quit | Command::Exit | Command::Interrupt => return Ok(true),
            Command::Help => {
                cmd::help::print_help();
            }
            Command::List => exec::<cmd::list::Args>(self, &args)?,
            _ => {}
        }

        Ok(false)
    }

    /// Executes a solang input
    fn on_solidty_input(&mut self, _unit: SourceUnit) -> eyre::Result<()> {
        Ok(())
    }

    /// Handle an unresolved input
    fn on_unknown(&mut self, s: String) -> eyre::Result<()> {
        eyre::bail!("unknown input: {:?}, try \"help\"", s)
    }

    /// Runs the next instruction
    ///
    /// Returns `true` if we should exit the shell
    fn run_once(&mut self) -> eyre::Result<bool> {
        let line = self.readline();
        debug!("Received line: {:?}", line);
        match line {
            Some(Input::Command(cmd, args)) => return self.run_cmd(cmd, args),
            Some(Input::Solang(unit)) => self.on_solidty_input(unit)?,
            Some(Input::Other(other)) => self.on_unknown(other)?,
            None => (),
        }
        Ok(false)
    }
}

fn exec<T: Cmd>(shell: &mut Shell, args: &[String]) -> eyre::Result<()> {
    T::run_str(shell, args)
}

#[derive(Debug)]
pub struct SignalRegister(AtomicUsize);

impl Default for SignalRegister {
    fn default() -> Self {
        SignalRegister(AtomicUsize::new(1))
    }
}

impl SignalRegister {
    pub fn catch_ctrl(&self) {
        self.0.store(0, Ordering::SeqCst);
    }

    pub fn add_ctrlc(&self) -> usize {
        self.0.fetch_add(1, Ordering::SeqCst)
    }

    pub fn ctrlc_received(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 1
    }

    pub fn reset_ctrlc(&self) {
        self.0.store(1, Ordering::SeqCst);
    }
}

/// Starts and runs the shell until exited
pub fn run(args: Args, config: Config) -> eyre::Result<()> {
    term::print_banner();
    let mut shell = Shell::new(args, config)?;
    shell.load_history().ok();
    shell.set_signal_handler().wrap_err("Failed to set signal handler")?;

    loop {
        match shell.run_once() {
            Ok(true) => break,
            Ok(_) => (),
            Err(err) => {
                term::error(&err.to_string());
                eprintln!("{}", err)
            }
        }
    }

    shell.save_history()?;

    Ok(())
}
