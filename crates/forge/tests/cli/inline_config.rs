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

#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(config_inline_isolate, |prj, cmd| {
    use serde::{Deserialize, Deserializer};
    use std::{fs, path::Path};

    prj.wipe_contracts();
    prj.add_test(
        "inline.sol",
        r#"
        import {Test} from "forge-std/Test.sol";

        contract Dummy {
            uint256 public number;

            function setNumber(uint256 newNumber) public {
                number = newNumber;
            }
        }

        contract FunctionConfig is Test {
            Dummy dummy;

            function setUp() public {
                dummy = new Dummy();
            }

            /// forge-config: default.isolate = true
            function test_isolate() public {
                vm.startSnapshotGas("testIsolatedFunction");
                dummy.setNumber(1);
                vm.stopSnapshotGas();
            }

            function test_non_isolate() public {
                vm.startSnapshotGas("testNonIsolatedFunction");
                dummy.setNumber(2);
                vm.stopSnapshotGas();
            }
        }

        /// forge-config: default.isolate = true
        contract ContractConfig is Test {
            Dummy dummy;

            function setUp() public {
                dummy = new Dummy();
            }

            function test_non_isolate() public {
                vm.startSnapshotGas("testIsolatedContract");
                dummy.setNumber(3);
                vm.stopSnapshotGas();
            }
        }
    "#,
    )
    .unwrap();

    cmd.args(["test", "-j1"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/inline.sol:ContractConfig
[PASS] test_non_isolate() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 2 tests for test/inline.sol:FunctionConfig
[PASS] test_isolate() ([GAS])
[PASS] test_non_isolate() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 3 tests passed, 0 failed, 0 skipped (3 total tests)

"#]]);

    assert!(prj.root().join("snapshots/FunctionConfig.json").exists());
    assert!(prj.root().join("snapshots/ContractConfig.json").exists());

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FunctionConfig {
        #[serde(deserialize_with = "string_to_u64")]
        test_isolated_function: u64,

        #[serde(deserialize_with = "string_to_u64")]
        test_non_isolated_function: u64,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ContractConfig {
        #[serde(deserialize_with = "string_to_u64")]
        test_isolated_contract: u64,
    }

    fn string_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: serde_json::Value = Deserialize::deserialize(deserializer)?;
        match s {
            serde_json::Value::String(s) => s.parse::<u64>().map_err(serde::de::Error::custom),
            serde_json::Value::Number(n) if n.is_u64() => Ok(n.as_u64().unwrap()),
            _ => Err(serde::de::Error::custom("Expected a string or number")),
        }
    }

    fn read_snapshot<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
        let content = fs::read_to_string(path).expect("Failed to read file");
        serde_json::from_str(&content).expect("Failed to parse snapshot")
    }

    let function_config: FunctionConfig =
        read_snapshot(&prj.root().join("snapshots/FunctionConfig.json"));
    let contract_config: ContractConfig =
        read_snapshot(&prj.root().join("snapshots/ContractConfig.json"));

    // FunctionConfig {
    //     test_isolated_function: 48926,
    //     test_non_isolated_function: 27722,
    // }

    // ContractConfig {
    //     test_isolated_contract: 48926,
    // }

    assert!(function_config.test_isolated_function > function_config.test_non_isolated_function);
    assert_eq!(function_config.test_isolated_function, contract_config.test_isolated_contract);
});

forgetest_init!(config_inline_evm_version, |prj, cmd| {
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
