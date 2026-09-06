use foundry_compilers::PathStyle;
use foundry_test_utils::TestProject;
use rexpect::{reader::Options, session::PtySession, spawn_with_options};

// Some inputs compile and evaluate code before the next prompt is printed; 3s is tight on CI.
const TIMEOUT_SECS: u64 = 10;
const PROMPT: &str = "➜ ";

/// Testing session for Chisel.
pub struct ChiselSession {
    session: Box<PtySession>,
    project: Box<TestProject>,
    is_repl: bool,
    /// Unread output from the last completed synchronous command.
    output: Option<String>,
}

static SUBCOMMANDS: &[&str] = &["list", "load", "view", "clear-cache", "eval", "help"];

fn is_repl(args: &[String]) -> bool {
    args.is_empty()
        || !SUBCOMMANDS.iter().any(|subcommand| args.iter().any(|arg| arg == subcommand))
}

#[allow(dead_code)]
impl ChiselSession {
    pub fn new(name: &str, flags: &str, init: bool) -> Self {
        let project = foundry_test_utils::TestProject::new(name, PathStyle::Dapptools);
        if init {
            foundry_test_utils::util::initialize(project.root());
            project.initialize_default_contracts();
        }

        let bin = env!("CARGO_BIN_EXE_chisel");
        let mut command = std::process::Command::new(bin);

        // TODO: TTY works but logs become unreadable.
        command.current_dir(project.root());
        command.env("NO_COLOR", "1");
        command.env("TERM", "dumb");

        command.env("ETHERSCAN_API_KEY", foundry_test_utils::rpc::next_etherscan_api_key());

        if !flags.is_empty() {
            command.args(flags.split_whitespace());
        }
        let args = command.get_args().map(|s| s.to_str().unwrap().to_string()).collect::<Vec<_>>();

        let session = spawn_with_options(
            command,
            Options {
                timeout_ms: Some(TIMEOUT_SECS * 1000),
                strip_ansi_escape_codes: false,
                encoding: rexpect::Encoding::UTF8,
            },
        )
        .unwrap();

        let is_repl = is_repl(&args);
        let mut session =
            Self { session: Box::new(session), project: Box::new(project), is_repl, output: None };

        // Expect initial prompt only if we're in the REPL.
        if session.is_repl() {
            session.expect("Welcome to Chisel!");
            session.expect_prompt();
        }

        session
    }

    pub fn project(&self) -> &TestProject {
        &self.project
    }

    pub const fn is_repl(&self) -> bool {
        self.is_repl
    }

    /// Send one complete REPL input and wait for its next prompt.
    ///
    /// Retain the command's output for subsequent `expect` calls. Send multiple independent
    /// inputs separately, or use `sendln_raw` and explicitly consume their prompts.
    #[track_caller]
    pub fn sendln(&mut self, line: &str) {
        self.sendln_raw(line);
        if self.is_repl() {
            self.output = Some(self.read_until_prompt());
        }
    }

    /// Send a line to the REPL without expecting the prompt to appear.
    ///
    /// You might want to call `expect_prompt` after this.
    #[track_caller]
    pub fn sendln_raw(&mut self, line: &str) {
        self.output = None;
        match self.session.send_line(line) {
            Ok(_) => (),
            Err(e) => {
                panic!("failed to send line {line:?}: {e}")
            }
        }
    }

    /// Expect the needle to appear.
    #[track_caller]
    pub fn expect(&mut self, needle: &str) {
        if let Some(output) = &mut self.output {
            let Some(pos) = output.find(needle) else {
                panic!("expected {needle:?} in completed command output: {output:?}");
            };
            output.drain(..pos + needle.len());
            return;
        }
        match self.session.exp_string(needle) {
            Ok(_) => (),
            Err(e) => {
                panic!("failed to expect {needle:?}: {e}")
            }
        }
    }

    /// Expect the prompt to appear.
    #[track_caller]
    pub fn expect_prompt(&mut self) {
        self.read_until_prompt();
    }

    #[track_caller]
    fn read_until_prompt(&mut self) -> String {
        self.session.exp_string(PROMPT).unwrap_or_else(|e| panic!("failed to expect prompt: {e}"))
    }

    /// Expect the prompt to appear `n` times.
    #[track_caller]
    pub fn expect_prompts(&mut self, n: usize) {
        for _ in 0..n {
            self.expect_prompt();
        }
    }
}
