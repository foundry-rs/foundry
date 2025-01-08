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
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Analysing contracts...
Running tests...

Ran 2 tests for test/inline.sol:Inline
[PASS] test1(bool) (runs: 2, [AVG_GAS])
[PASS] test2(bool) (runs: 3, [AVG_GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

╭-------+---------------+---------------+---------------+---------------╮
| File  | % Lines       | % Statements  | % Branches    | % Funcs       |
+=======================================================================+
| Total | 100.00% (0/0) | 100.00% (0/0) | 100.00% (0/0) | 100.00% (0/0) |
╰-------+---------------+---------------+---------------+---------------╯

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
Error: Inline config error at test/inline.sol:80:123:0: invalid profile `unknown.fuzz.runs = 2`; valid profiles: default

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
[FAIL: invalid type: found sequence, expected u32 for key "default.fuzz.runs" in inline config] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/inline.sol:Inline
[FAIL: invalid type: found sequence, expected u32 for key "default.fuzz.runs" in inline config] setUp() ([GAS])

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
[FAIL: invalid type: found string "2", expected u32 for key "default.fuzz.runs" in inline config] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/inline.sol:Inline
[FAIL: invalid type: found string "2", expected u32 for key "default.fuzz.runs" in inline config] setUp() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

forgetest_init!(evm_version, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "inline.sol",
        r#"
        import {Test} from "forge-std/Test.sol";

        contract Dummy {
            function getBlobBaseFee() public returns (uint256) {
                return block.blobbasefee;
            }
        }

        contract FunctionConfig is Test {
            Dummy dummy;

            function setUp() public {
                dummy = new Dummy();
            }

            /// forge-config: default.evm_version = "shanghai"
            function test_old() public {
                vm.expectRevert();
                dummy.getBlobBaseFee();
            }

            function test_new() public {
                dummy.getBlobBaseFee();
            }
        }

        /// forge-config: default.evm_version = "shanghai"
        contract ContractConfig is Test {
            Dummy dummy;

            function setUp() public {
                dummy = new Dummy();
            }

            function test_old() public {
                vm.expectRevert();
                dummy.getBlobBaseFee();
            }

            /// forge-config: default.evm_version = "cancun"
            function test_new() public {
                dummy.getBlobBaseFee();
            }
        }
    "#,
    )
    .unwrap();

    cmd.args(["test", "--evm-version=cancun", "-j1"]).assert_success().stdout_eq(str![[r#"
...
Ran 2 tests for test/inline.sol:ContractConfig
[PASS] test_new() ([GAS])
[PASS] test_old() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 2 tests for test/inline.sol:FunctionConfig
[PASS] test_new() ([GAS])
[PASS] test_old() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)

"#]]);
});
