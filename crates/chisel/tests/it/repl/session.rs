use foundry_compilers::PathStyle;
use foundry_test_utils::TestProject;
use std::{io, time::Duration};

type ReplSession = expectrl::repl::ReplSession<
    expectrl::process::unix::UnixProcess,
    expectrl::stream::log::LogStream<expectrl::process::unix::AsyncPtyStream, LogWriter>,
>;

const TIMEOUT_SECS: u64 = 3;

pub struct ChiselSession {
    session: Box<ReplSession>,
    project: Box<TestProject>,
}

static SUBCOMMANDS: &[&str] = &["list", "load", "view", "clear-cache", "eval", "help"];

fn is_repl(args: &[String]) -> bool {
    args.is_empty()
        || !SUBCOMMANDS.iter().any(|subcommand| args.iter().any(|arg| arg == subcommand))
}

#[allow(dead_code)]
impl ChiselSession {
    pub async fn new(name: &str, flags: &str, init: bool) -> Self {
        let project = foundry_test_utils::TestProject::new(name, PathStyle::Dapptools);
        if init {
            foundry_test_utils::util::initialize(project.root());
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

        let session = expectrl::Session::spawn(command).unwrap();
        let log = LogWriter;
        let session = expectrl::session::log(session, log).unwrap();
        let mut repl: ReplSession = expectrl::repl::ReplSession::new(
            session,
            "➜ ".to_string(),
            Some("!q".to_string()),
            false,
        );
        repl.set_expect_timeout(Some(Duration::from_secs(TIMEOUT_SECS)));

        let mut repl = Self { session: Box::new(repl), project: Box::new(project) };

        // Expect initial prompt only if we're in the REPL.
        if is_repl(&args) {
            repl.expect("Welcome to Chisel!").await;
        }

        repl
    }

    pub fn project(&self) -> &TestProject {
        &self.project
    }

    /// Send a line to the REPL and expects the prompt to appear.
    pub async fn sendln(&mut self, line: &str) {
        match self.session.execute(line).await {
            Ok(_) => (),
            Err(e) => {
                panic!("failed to send line {line:?}: {e}")
            }
        }
    }

    /// Send a line to the REPL without expecting the prompt to appear.
    ///
    /// You might want to call `expect_prompt` after this.
    pub async fn sendln_raw(&mut self, line: &str) {
        match self.session.send_line(line).await {
            Ok(_) => (),
            Err(e) => {
                panic!("failed to send line {line:?}: {e}")
            }
        }
    }

    /// Expect the needle to appear.
    pub async fn expect(&mut self, needle: &str) {
        match self.session.expect(needle).await {
            Ok(_) => (),
            Err(e) => {
                panic!("failed to expect {needle:?}: {e}")
            }
        }
    }

    /// Expect the prompt to appear.
    pub async fn expect_prompt(&mut self) {
        let prompt = self.session.get_prompt().to_string();
        self.expect(&prompt).await;
    }

    /// Expect the prompt to appear `n` times.
    pub async fn expect_prompts(&mut self, n: usize) {
        let prompt = self.session.get_prompt().to_string();
        for _ in 0..n {
            self.expect(&prompt).await;
        }
    }
}

struct LogWriter;
impl io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // The main difference between `stderr`: use the `eprint!` macro so that the output
        // can get captured by the test harness.
        eprint!("{}", String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.write(buf).map(drop)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stderr().flush()
    }
}
