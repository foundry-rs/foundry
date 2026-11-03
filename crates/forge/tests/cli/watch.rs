#[cfg(unix)]
use foundry_test_utils::{assert_data_eq, str};
#[cfg(unix)]
use rexpect::{
    Encoding,
    process::{signal, wait::WaitStatus},
    reader::Options,
    spawn_with_options,
};
#[cfg(unix)]
use std::time::Duration;

#[forgetest]
#[cfg(unix)]
fn watch_yields_stdin_to_tests(prj: _) {
    const TIMEOUT_MS: u64 = 30_000;

    let file = prj.add_test(
        "Prompt.t.sol",
        r#"
interface Vm {
    function prompt(string calldata promptText) external returns (string memory input);
}

contract PromptTest {
    Vm private constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testBefore() public {}

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
        .args(["test", "--watch", "--no-restart"]);

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
    // The running command has already compiled the old inputs. Discard this event while it waits.
    let edited = std::fs::read_to_string(&file).unwrap().replace("testBefore", "testEdited");
    std::fs::write(&file, &edited).unwrap();
    std::thread::sleep(Duration::from_secs(1));
    session.send_line("ok").unwrap();
    session.exp_string("[PASS] testPrompt()").unwrap();
    session.exp_regex(r"\[Command was successful[^\n]*\n").unwrap();

    // Saving the discarded edit after completion must still execute the new inputs.
    std::fs::write(&file, &edited).unwrap();
    session.exp_string("Enter ok:").unwrap();
    session.send_line("ok").unwrap();
    session.exp_string("[PASS] testEdited()").unwrap();
    session.exp_regex(r"\[Command was successful[^\n]*\n").unwrap();

    session.process.signal(signal::SIGINT).unwrap();
    let output = session.exp_eof().unwrap();
    assert!(
        matches!(session.process.wait().unwrap(), WaitStatus::Exited(_, 0)),
        "watch command failed: {output}"
    );
}

#[forgetest]
#[cfg(unix)]
fn watch_skips_unchanged_saves_and_detects_restored_files(prj: _) {
    let file =
        prj.add_test("Watch.t.sol", "contract WatchTest { function testInitial() public {} }");
    let initial = std::fs::read_to_string(&file).unwrap();
    let edited = initial.replace("testInitial", "testEdited");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_forge"));
    command
        .current_dir(prj.root())
        .env_remove("CI")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        // Relative roots must seed the same keys as the watcher's absolute event paths.
        .args(["test", "--watch", "test", "--run-all"]);
    let mut session = spawn_with_options(
        command,
        Options {
            timeout_ms: Some(30_000),
            strip_ansi_escape_codes: true,
            encoding: Encoding::UTF8,
        },
    )
    .unwrap();

    session.exp_string("[PASS] testInitial()").unwrap();
    session.exp_regex(r"\[Command was successful[^\n]*\n").unwrap();

    // Synchronize with the completed initial command before checking for an unwanted rerun.
    std::fs::write(&file, &initial).unwrap();
    std::thread::sleep(Duration::from_secs(1));
    let output = std::iter::from_fn(|| session.try_read()).collect::<String>();
    assert_data_eq!(output, str![""]);

    // Editors also save atomically via a temporary path that no longer exists at debounce time.
    let temporary = file.with_extension("sol.tmp");
    std::fs::write(&temporary, &initial).unwrap();
    std::fs::rename(&temporary, &file).unwrap();
    std::thread::sleep(Duration::from_secs(1));
    let output = std::iter::from_fn(|| session.try_read()).collect::<String>();
    assert_data_eq!(output, str![""]);

    // Skipping a filesystem no-op must not suppress an explicit keyboard rerun.
    session.send("a").unwrap();
    session.flush().unwrap();
    session.exp_string("[PASS] testInitial()").unwrap();
    session.exp_regex(r"\[Command was successful[^\n]*\n").unwrap();

    std::fs::write(&file, &edited).unwrap();
    session.exp_string("[PASS] testEdited()").unwrap();
    session.exp_regex(r"\[Command was successful[^\n]*\n").unwrap();

    std::fs::remove_file(&file).unwrap();
    session.exp_regex(r"\[Command was successful[^\n]*\n").unwrap();
    std::fs::write(&file, &edited).unwrap();
    session.exp_string("[PASS] testEdited()").unwrap();
    session.exp_regex(r"\[Command was successful[^\n]*\n").unwrap();

    // A FIFO is a valid filesystem event, but opening it for reading would hang the watcher.
    let fifo = prj.root().join("test/request");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    session.exp_string("[PASS] testEdited()").unwrap();
    session.exp_regex(r"\[Command was successful[^\n]*\n").unwrap();

    session.process.signal(signal::SIGINT).unwrap();
    let output = session.exp_eof().unwrap();
    assert!(
        matches!(session.process.wait().unwrap(), WaitStatus::Exited(_, 0)),
        "watch command failed: {output}"
    );
}
