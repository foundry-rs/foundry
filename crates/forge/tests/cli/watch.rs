#[cfg(unix)]
use rexpect::{
    Encoding,
    process::{signal, wait::WaitStatus},
    reader::Options,
    spawn_with_options,
};

forgetest!(
    #[cfg(unix)]
    watch_yields_stdin_to_tests,
    |prj, _cmd| {
        const TIMEOUT_MS: u64 = 30_000;

        prj.add_test(
            "Prompt.t.sol",
            r#"
interface Vm {
    function prompt(string calldata promptText) external returns (string memory input);
}

contract PromptTest {
    Vm private constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testPrompt() public {
        string memory input = vm.prompt("Enter ok");
        require(keccak256(bytes(input)) == keccak256("ok"), "unexpected input");
    }
}
"#,
        );

        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_forge"));
        command
            .current_dir(prj.root())
            .env_remove("CI")
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .args(["test", "--watch", "--match-test", "testPrompt"]);

        let mut session = spawn_with_options(
            command,
            Options {
                timeout_ms: Some(TIMEOUT_MS),
                strip_ansi_escape_codes: true,
                encoding: Encoding::UTF8,
            },
        )
        .unwrap();

        session.exp_string("Enter ok:").unwrap();
        session.send_line("ok").unwrap();
        session.exp_string("[PASS] testPrompt()").unwrap();
        session.exp_string("[Command was successful").unwrap();

        session.process.signal(signal::SIGINT).unwrap();
        let output = session.exp_eof().unwrap();
        assert!(
            matches!(session.process.wait().unwrap(), WaitStatus::Exited(_, 0)),
            "watch command failed: {output}"
        );
    }
);
