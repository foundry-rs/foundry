//! Contains various tests for checking the debugger
use commands::*;
use foundry_cli_test_utils::{
    forgetest, forgetest_init,
    stdin::StdInCommand,
    util::{TestCommand, TestProject},
};

const TEST_EXAMPLE_SIG: &str = "testExample()";

/// Contains various keybinding for interacting with the debugger via stdin
mod commands {
    fn quit() -> StdInCommand {
        "q".into()
    }
    fn down() -> StdInCommand {
        "j".into()
    }
    fn up() -> StdInCommand {
        "k".into()
    }
    fn top_of_file() -> StdInCommand {
        "g".into()
    }
    fn bottom_of_file() -> StdInCommand {
        "G".into()
    }
    fn next_call() -> StdInCommand {
        "C".into()
    }
    fn forward() -> StdInCommand {
        "s".into()
    }
    fn backwards() -> StdInCommand {
        "a".into()
    }
}

// tests that we can run the debugger for the template function
forgetest_init!(can_start_debugger_for_function, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["test", "--debug", TEST_EXAMPLE_SIG]);

    let out = cmd.spawn_and_send_stdin([quit()]);
    println!("{}", out);
});
