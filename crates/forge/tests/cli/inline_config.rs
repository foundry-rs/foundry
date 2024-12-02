forgetest!(runs, |prj, cmd| {
    prj.add_test(
        "inline.sol",
        "
        contract Inline {
                /** forge-config:  default.fuzz.runs = 2 */
            function test1(bool) public {}

            \t///\t forge-config:\tdefault.fuzz.runs=\t3 \t

            function test2(bool) public {}
        }
    ",
    )
    .unwrap();

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/inline.sol:Inline
[PASS] test1(bool) (runs: 2, [AVG_GAS])
[PASS] test2(bool) (runs: 3, [AVG_GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);

    // Make sure inline config is parsed in coverage too.
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
Ran 2 tests for test/inline.sol:Inline
[PASS] test1(bool) (runs: 2, [AVG_GAS])
[PASS] test2(bool) (runs: 3, [AVG_GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)
| File  | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------|---------------|---------------|---------------|---------------|
| Total | 100.00% (0/0) | 100.00% (0/0) | 100.00% (0/0) | 100.00% (0/0) |

"#]]);
});

forgetest!(invalid_profile, |prj, cmd| {
    prj.add_test(
        "inline.sol",
        "
        /** forge-config:  unknown.fuzz.runs = 2 */
        contract Inline {
            function test(bool) public {}
        }
    ",
    )
    .unwrap();

    cmd.arg("test").assert_failure().stderr_eq(str![[r#"
Error: Inline config error at test/inline.sol:0:0:0: invalid profile `unknown.fuzz.runs = 2`; valid profiles: default

"#]]);
});

// TODO: Uncomment once this done for normal config too.
/*
forgetest!(invalid_key, |prj, cmd| {
    prj.add_test(
        "inline.sol",
        "
        /** forge-config:  default.fuzzz.runs = 2 */
        contract Inline {
            function test(bool) public {}
        }
    ",
    )
    .unwrap();

    cmd.arg("test").assert_failure().stderr_eq(str![[]]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/inline.sol:Inline
[FAIL: failed to get inline configuration: unknown config section `default`] test(bool) ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/inline.sol:Inline
[FAIL: failed to get inline configuration: unknown config section `default`] test(bool) ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

forgetest!(invalid_key_2, |prj, cmd| {
    prj.add_test(
        "inline.sol",
        "
/** forge-config:  default.fuzz.runss = 2 */
        contract Inline {
            function test(bool) public {}
        }
    ",
    )
    .unwrap();

    cmd.arg("test").assert_failure().stderr_eq(str![[]]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/inline.sol:Inline
[FAIL: failed to get inline configuration: unknown config section `default`] test(bool) ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/inline.sol:Inline
[FAIL: failed to get inline configuration: unknown config section `default`] test(bool) ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});
*/

forgetest!(invalid_value, |prj, cmd| {
    prj.add_test(
        "inline.sol",
        "
        /** forge-config:  default.fuzz.runs = [2] */
        contract Inline {
            function test(bool) public {}
        }
    ",
    )
    .unwrap();

    cmd.arg("test").assert_failure().stderr_eq(str![[]]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/inline.sol:Inline
[FAIL: invalid type: found sequence, expected u32 for key "default.runs.fuzz" in inline config] test(bool) ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/inline.sol:Inline
[FAIL: invalid type: found sequence, expected u32 for key "default.runs.fuzz" in inline config] test(bool) ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

forgetest!(invalid_value_2, |prj, cmd| {
    prj.add_test(
        "inline.sol",
        "
        /** forge-config:  default.fuzz.runs = '2' */
        contract Inline {
            function test(bool) public {}
        }
    ",
    )
    .unwrap();

    cmd.arg("test").assert_failure().stderr_eq(str![[]]).stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/inline.sol:Inline
[FAIL: invalid type: found string "2", expected u32 for key "default.runs.fuzz" in inline config] test(bool) ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/inline.sol:Inline
[FAIL: invalid type: found string "2", expected u32 for key "default.runs.fuzz" in inline config] test(bool) ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});
