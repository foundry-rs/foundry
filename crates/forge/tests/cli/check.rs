use foundry_test_utils::{forgetest, str};

// Test that `forge check` succeeds on a valid project
forgetest_init!(check_basic, |prj, cmd| {
    cmd.args(["check"]).assert_success().stdout_eq(str![[r#"
Check completed successfully

"#]]);
});

// Test that `forge check` detects syntax errors
forgetest!(check_syntax_error, |prj, cmd| {
    prj.add_source(
        "SyntaxError",
        r"
contract SyntaxError {
    uint256 public value = 10  // missing semicolon

    function test() public {}
}
",
    );

    cmd.args(["check"]).assert_failure().stderr_eq(str![["
error: expected one of `(`, `.`, `;`, `?`, `[`, or `{`, found keyword `function`
[..]
[..]
[..]
[..]
[..]
[..]
[..]

Error: Check failed for Solidity version [..]

"]]);
});

// Test that `forge check` detects undefined symbols
forgetest!(check_undefined_symbol, |prj, cmd| {
    prj.add_source(
        "UndefinedVar",
        r"
contract UndefinedVar {
    function test() public pure returns (uint256) {
        return undefinedVariable;
    }
}
",
    );

    cmd.args(["check"]).assert_failure().stderr_eq(str![["
error: unresolved symbol `undefinedVariable`
[..]
[..]
[..]
[..]

Error: Check failed for Solidity version [..]

"]]);
});

// Test that `forge check` can check specific paths
forgetest!(check_specific_path, |prj, cmd| {
    prj.add_source(
        "Good",
        r"
contract Good {
    uint256 public value;
}
",
    );

    prj.add_source(
        "Bad",
        r"
contract Bad {
    uint256 public value = 10  // missing semicolon
}
",
    );

    // Check only the good file should succeed
    cmd.args(["check", "src/Good.sol"]).assert_success().stdout_eq(str![[r#"
Check completed successfully

"#]]);
});
