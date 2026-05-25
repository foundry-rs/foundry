//! Binary-level contract tests for the `--machine` agent runtime.

use foundry_test_utils::{forgetest, forgetest_init};
use serde_json::Value;

/// Returns true if the byte slice contains a CSI escape (`ESC [`).
fn has_ansi(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|w| w == [0x1b, b'['])
}

/// Parses the stdout as a top-level envelope and asserts no ANSI escapes leaked.
fn parse_envelope(stdout: &[u8]) -> Value {
    assert!(!has_ansi(stdout), "envelope leaked ANSI escapes");
    serde_json::from_slice(stdout).unwrap_or_else(|e| {
        panic!("stdout was not JSON: {e}\n----\n{}\n----", String::from_utf8_lossy(stdout))
    })
}

forgetest!(machine_version_emits_success_envelope, |_prj, cmd| {
    let out = cmd.args(["--machine", "--version"]).assert_success().get_output().stdout.clone();
    let env = parse_envelope(&out);
    assert_eq!(env["success"], true);
    assert!(env["data"]["version"].is_string());
});

forgetest!(machine_help_emits_success_envelope, |_prj, cmd| {
    let out = cmd.args(["--machine", "--help"]).assert_success().get_output().stdout.clone();
    let env = parse_envelope(&out);
    assert_eq!(env["success"], true);
    assert!(env["data"]["help"].is_string());
});

forgetest!(machine_unknown_flag_emits_usage_envelope, |_prj, cmd| {
    let out = cmd.args(["--machine", "--badflag"]).assert_code(2).get_output().stdout.clone();
    let env = parse_envelope(&out);
    assert_eq!(env["success"], false);
    assert_eq!(env["errors"][0]["code"], "cli.usage.invalid");
});

// `<subcommand> --help` under `--machine` must surface the subcommand's own help text.
forgetest!(machine_subcommand_help_preserves_context, |_prj, cmd| {
    let out =
        cmd.args(["--machine", "build", "--help"]).assert_success().get_output().stdout.clone();
    let env = parse_envelope(&out);
    let help = env["data"]["help"].as_str().expect("help is a string");
    assert!(help.contains("Build the project's smart contracts"), "leaked root help: {help}");
});

// `--machine` after a subcommand must also flip machine mode (clap-global).
forgetest!(machine_flag_honored_after_subcommand, |_prj, cmd| {
    let out =
        cmd.args(["build", "--machine", "--help"]).assert_success().get_output().stdout.clone();
    let env = parse_envelope(&out);
    assert_eq!(env["success"], true);
    let help = env["data"]["help"].as_str().expect("help is a string");
    assert!(help.contains("Build the project's smart contracts"), "leaked root help: {help}");
});

forgetest!(machine_conflicts_with_json, |_prj, cmd| {
    let out = cmd.args(["--machine", "--json", "build"]).assert_code(2).get_output().stdout.clone();
    let env = parse_envelope(&out);
    assert_eq!(env["errors"][0]["code"], "cli.usage.invalid");
});

forgetest!(machine_conflicts_with_md, |_prj, cmd| {
    let out = cmd.args(["--machine", "--md", "build"]).assert_code(2).get_output().stdout.clone();
    let env = parse_envelope(&out);
    assert_eq!(env["errors"][0]["code"], "cli.usage.invalid");
});

static FAILING_TEST: &str = r#"
import "forge-std/Test.sol";

contract MachineFailingTest is Test {
    function testShouldFail() public {
        assertTrue(false);
    }
}
"#;

// Failing tests: legacy exits 1, `--machine` exits 5 (`ExitCode::TestFailure`).
forgetest_init!(machine_test_failure_uses_canonical_exit_code, |prj, cmd| {
    prj.add_source("machine_failing_test", FAILING_TEST);

    cmd.forge_fuse().args(["test"]).assert_code(1);
    cmd.forge_fuse().args(["--machine", "test"]).assert_code(5);
});
