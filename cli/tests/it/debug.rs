//! Contains various tests for checking the debugger
use commands::*;
use foundry_cli_test_utils::{
    forgetest, forgetest_ignore, forgetest_init,
    util::{TestCommand, TestProject},
};

const TEST_EXAMPLE_SIG: &str = "testExample()";

/// Contains various keybinding for interacting with the debugger via stdin
#[allow(unused)]
mod commands {
    use foundry_cli_test_utils::stdin::StdInKeyCommand;

    pub fn quit() -> StdInKeyCommand {
        'q'.into()
    }
    pub fn down() -> StdInKeyCommand {
        'j'.into()
    }
    pub fn up() -> StdInKeyCommand {
        'k'.into()
    }
    pub fn top_of_file() -> StdInKeyCommand {
        'g'.into()
    }
    pub fn bottom_of_file() -> StdInKeyCommand {
        'G'.into()
    }
    pub fn next_call() -> StdInKeyCommand {
        'C'.into()
    }
    pub fn forward() -> StdInKeyCommand {
        's'.into()
    }
    pub fn backwards() -> StdInKeyCommand {
        'a'.into()
    }
}

// tests that we can run the debugger for the template function
forgetest_ignore!(can_start_debugger_for_function, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("build");
    cmd.assert_non_empty_stdout();
    cmd.forge_fuse().args(["test", "--debug", TEST_EXAMPLE_SIG]).root_arg();

    let _out = cmd.spawn_and_send_stdin([quit()]);
});
