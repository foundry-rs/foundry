use foundry_test_utils::{
    TestCommand,
    snapbox::{Data, IntoData, assert_data_eq, cmd::OutputAssert},
};
use std::process::Command;

pub fn z3_available() -> bool {
    Command::new("z3").arg("--version").output().is_ok_and(|output| output.status.success())
}

#[macro_export]
macro_rules! skip_unless_z3 {
    ($name:literal) => {
        if !$crate::test_cmd::symbolic_helpers::z3_available() {
            let _ = foundry_common::sh_eprintln!("skipping {} because z3 is not available", $name);
            return;
        }
    };
}

/// Run a symbolic test with redactions that mask solver-dependent / wall-clock
/// noise so the snapshot is stable across solver versions and runs.
///
/// - `[METRICS]` — symbolic metrics line suffix (engine internal metrics change with solver
///   heuristic / engine path-pruning changes).
/// - `[SENDER]` — `sender=0x...` symbolic invariant senders, which the solver picks freely from an
///   unconstrained address pool.
pub fn assert_symbolic(cmd: &mut TestCommand) -> OutputAssert {
    cmd.assert_with(&[
        ("[METRICS]", r"(?:paths: \d+, queries: \d+(?:, smt: \d+, sat: \d+ \(\d+ cached\), models: \d+ \(\d+ cached\), hard-arith: \d+, solver: \d+ms)?|runs: \d+, calls: \d+, reverts: \d+)"),
        ("[SENDER]", r"sender=0x[0-9a-fA-F]{40}"),
    ])
}

/// Same as [`assert_symbolic`], plus redactions for counterexample witnesses
/// whose exact values Z3 chooses freely (calldata bytes, args list, raw
/// addresses inside args). Use for tests whose property only asserts that
/// *some* counterexample exists, not what it is.
pub fn assert_symbolic_witness(cmd: &mut TestCommand) -> OutputAssert {
    cmd.assert_with(&[
        ("[METRICS]", r"(?:paths: \d+, queries: \d+(?:, smt: \d+, sat: \d+ \(\d+ cached\), models: \d+ \(\d+ cached\), hard-arith: \d+, solver: \d+ms)?|runs: \d+, calls: \d+, reverts: \d+)"),
        ("[SENDER]", r"sender=0x[0-9a-fA-F]{40}"),
        ("[CALLDATA]", r"calldata=0x[0-9a-fA-F]+"),
        // `args=[...]` may contain nested scientific-notation brackets like
        // `args=[1234 [1.2e3], 5678 [5.6e3]]`, so allow one level of nesting.
        ("[ARGS]", r"args=\[(?:[^\[\]]|\[[^\]]*\])*\]"),
    ])
}

pub fn assert_relevant_lines(stdout: &str, expected: impl IntoData) {
    let expected = expected.into_data();
    let expected_lines = expected.to_string();
    let mut actual = String::new();

    for expected_line in expected_lines.lines().filter(|line| !line.is_empty()) {
        stdout
            .lines()
            .find(|line| line.contains(expected_line))
            .unwrap_or_else(|| panic!("missing line `{expected_line}` in stdout:\n{stdout}"));
        actual.push_str(expected_line);
        actual.push('\n');
    }

    assert_data_eq!(Data::from(actual.trim_end_matches('\n')), expected);
}
