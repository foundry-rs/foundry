#[forgetest]
fn runs(prj: _, cmd: _) {
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
    );

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

╭-------+-----------+--------------+------------+-----------╮
| File  | % Lines   | % Statements | % Branches | % Funcs   |
+===========================================================+
| Total | N/A (0/0) | N/A (0/0)    | N/A (0/0)  | N/A (0/0) |
╰-------+-----------+--------------+------------+-----------╯

"#]]);
}

#[forgetest]
fn invalid_profile(prj: _, cmd: _) {
    prj.add_test(
        "inline.sol",
        "
        /** forge-config:  unknown.fuzz.runs = 2 */
        contract Inline {
            function test(bool) public {}
        }
    ",
    );

    cmd.arg("test").assert_failure().stderr_eq(str![[r#"
Error: Inline config error at test/inline.sol:4:9: invalid profile `unknown.fuzz.runs = 2`; valid profiles: default

"#]]);
}

#[forgetest]
fn invalid_value(prj: _, cmd: _) {
    prj.add_test(
        "inline.sol",
        "
        /** forge-config:  default.fuzz.runs = [2] */
        contract Inline {
            function test(bool) public {}
        }
    ",
    );

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

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
}

#[forgetest]
fn invalid_value_2(prj: _, cmd: _) {
    prj.add_test(
        "inline.sol",
        "
        /** forge-config:  default.fuzz.runs = '2' */
        contract Inline {
            function test(bool) public {}
        }
    ",
    );

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

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
}

#[forgetest_init]
fn config_inline_isolate(prj: _, cmd: _) {
    use serde::{Deserialize, Deserializer};
    use std::{fs, path::Path};

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

            /// forge-config: default.isolate = false
            function test_non_isolate() public {
                assertFalse(vm.isIsolateMode());
                vm.startSnapshotGas("testNonIsolatedFunction");
                dummy.setNumber(1);
                vm.stopSnapshotGas();
            }

            function test_isolate() public {
                assertTrue(vm.isIsolateMode());
                vm.startSnapshotGas("testIsolatedFunction");
                dummy.setNumber(1);
                vm.stopSnapshotGas();
            }
        }

        /// forge-config: default.isolate = false
        contract ContractConfig is Test {
            Dummy dummy;

            function setUp() public {
                dummy = new Dummy();
            }

            function test_non_isolate() public {
                assertFalse(vm.isIsolateMode());
                vm.startSnapshotGas("testNonIsolatedContract");
                dummy.setNumber(1);
                vm.stopSnapshotGas();
            }
        }
    "#,
    );

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
        test_non_isolated_contract: u64,
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

    assert!(
        function_config.test_isolated_function > function_config.test_non_isolated_function,
        "isolated gas ({}) should be greater than non-isolated gas ({})",
        function_config.test_isolated_function,
        function_config.test_non_isolated_function
    );
    assert_eq!(
        function_config.test_non_isolated_function,
        contract_config.test_non_isolated_contract
    );
}

#[forgetest_init]
fn is_isolate_mode_uses_effective_isolation(prj: _, cmd: _) {
    prj.update_config(|config| config.isolate = false);
    prj.add_test(
        "effective_isolation.sol",
        r#"
        import {Test} from "forge-std/Test.sol";

        contract EffectiveIsolationTest is Test {
            function test_isolate_mode_disabled_by_config() public view {
                assertFalse(vm.isIsolateMode());
            }

            function test_gas_report_enables_isolate_mode() public view {
                assertTrue(vm.isIsolateMode());
            }
        }
    "#,
    );

    cmd.args(["test", "--match-test", "test_isolate_mode_disabled_by_config"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/effective_isolation.sol:EffectiveIsolationTest
[PASS] test_isolate_mode_disabled_by_config() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
    cmd.forge_fuse()
        .args(["test", "--gas-report", "--match-test", "test_gas_report_enables_isolate_mode"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/effective_isolation.sol:EffectiveIsolationTest
[PASS] test_gas_report_enables_isolate_mode() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn inline_isolate_inherits_default_fs_permissions_for_non_default_profile(prj: _, cmd: _) {
    std::fs::write(
        prj.root().join("foundry.toml"),
        r#"
        [profile.default]
        src = "src"
        out = "out"
        libs = ["lib"]
        fs_permissions = [{ access = "read-write", path = "./data" }]

        [profile.test]
        "#,
    )
    .unwrap();
    prj.add_test(
        "inline_isolate_fs_permissions.sol",
        r#"
        import {Test} from "forge-std/Test.sol";

        contract InlineIsolateFsPermissionsTest is Test {
            function setUp() public {
                if (!vm.exists("./data")) {
                    vm.createDir("./data", true);
                }
            }

            /// forge-config: default.isolate = true
            function testInlineIsolateCanCreateFile() public {
                vm.writeFile("./data/new.txt", "hello");
                vm.removeFile("./data/new.txt");
            }
        }
    "#,
    );

    cmd.env("FOUNDRY_PROFILE", "test");
    cmd.args(["test", "--match-test", "testInlineIsolateCanCreateFile"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/inline_isolate_fs_permissions.sol:InlineIsolateFsPermissionsTest
[PASS] testInlineIsolateCanCreateFile() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn config_inline_evm_version(prj: _, cmd: _) {
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
    );

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
}

#[forgetest_init]
fn config_inline_hardfork_same_network_family(prj: _, cmd: _) {
    prj.write_config(foundry_config::Config {
        hardfork: Some("tempo:T2".parse::<foundry_config::FoundryHardfork>().unwrap()),
        ..foundry_config::Config::default()
    });
    prj.add_test(
        "inline.sol",
        r#"
        import {Test} from "forge-std/Test.sol";

        contract InlineHardfork is Test {
            /// forge-config: default.hardfork = "tempo:T3"
            function test_inline_hardfork() public {
                assertTrue(true);
            }
        }
    "#,
    );

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/inline.sol:InlineHardfork
[PASS] test_inline_hardfork() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
}

// Checks that tests annotated with `forge-config: default.networks.network` run on the correct
// EVM network, and that unannotated tests run on the globally configured network.
//
// Each test makes a real call to the Tempo `TipFeeManager` precompile at
// `0xfeec000000000000000000000000000000000000` (a Tempo-only contract that exists on the
// Moderato testnet and is auto-injected by the in-memory Tempo EVM):
//
// * The default-network test asserts the precompile has no code (it does not exist on Ethereum).
// * The Tempo-network test asserts the precompile has code and `userTokens(address)` returns the
//   unset zero-address sentinel, proving the Tempo network was actually selected for that test and
//   the Tempo genesis state was loaded.
#[forgetest]
fn per_test_network_routing(prj: _, cmd: _) {
    prj.add_test(
        "inline.sol",
        r#"
        address constant TIP_FEE_MANAGER = 0xfeEC000000000000000000000000000000000000;

        contract DefaultNetwork {
            // No annotation -> runs on the globally selected network (Ethereum by default).
            // The Tempo FeeManager precompile must NOT exist here.
            function test_fee_manager_absent_on_ethereum() public view {
                require(
                    TIP_FEE_MANAGER.code.length == 0,
                    "TipFeeManager should not exist on Ethereum"
                );
            }
        }

        contract TempoNetwork {
            /// forge-config: default.networks.network = "tempo"
            function test_fee_manager_callable_on_tempo() public view {
                // Sentinel bytecode (0xef) is injected at every Tempo precompile address.
                require(
                    TIP_FEE_MANAGER.code.length > 0,
                    "TipFeeManager must be deployed on Tempo"
                );

                // Call a Tempo-only method: `userTokens(address)` returns the user's preferred
                // fee token, or the zero address when none is set.
                (bool ok, bytes memory ret) = TIP_FEE_MANAGER.staticcall(
                    abi.encodeWithSignature("userTokens(address)", address(0))
                );
                require(ok, "userTokens call to TipFeeManager failed");
                require(ret.length == 32, "unexpected return data length");
                address token = abi.decode(ret, (address));
                require(token == address(0), "expected unset user fee token");
            }
        }

        // Mixed contract: one function annotated with Tempo, one unannotated (runs on Ethereum).
        contract MixedNetwork {
            // No annotation -> runs on Ethereum; precompile must be absent.
            function test_fee_manager_absent_on_ethereum() public view {
                require(
                    TIP_FEE_MANAGER.code.length == 0,
                    "TipFeeManager should not exist on Ethereum"
                );
            }

            /// forge-config: default.networks.network = "tempo"
            function test_fee_manager_callable_on_tempo() public view {
                require(
                    TIP_FEE_MANAGER.code.length > 0,
                    "TipFeeManager must be deployed on Tempo"
                );

                (bool ok, bytes memory ret) = TIP_FEE_MANAGER.staticcall(
                    abi.encodeWithSignature("userTokens(address)", address(0))
                );
                require(ok, "userTokens call to TipFeeManager failed");
                require(ret.length == 32, "unexpected return data length");
                address token = abi.decode(ret, (address));
                require(token == address(0), "expected unset user fee token");
            }
        }
        "#,
    );

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/inline.sol:[..]Network
[PASS] test_fee_manager_absent_on_[..]() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/inline.sol:[..]Network
[PASS] test_fee_manager_absent_on_[..]() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/inline.sol:[..]Network
[PASS] test_fee_manager_callable_on_[..]() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/inline.sol:[..]Network
[PASS] test_fee_manager_callable_on_[..]() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 3 test suites [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)

"#]]);
}

// Contract-level network configuration must not override the execution profile selected for each
// multi-network pass. Function-level configuration can still select a different pass.
#[forgetest]
fn contract_network_preserves_pass_profile(prj: _, cmd: _) {
    prj.add_test(
        "inline.sol",
        r#"
        address constant TIP_FEE_MANAGER = 0xfeEC000000000000000000000000000000000000;

        /// forge-config: default.networks.network = "tempo"
        contract ContractNetwork {
            /// forge-config: default.networks.network = "ethereum"
            function test_fee_manager_absent_on_ethereum() public view {
                require(
                    TIP_FEE_MANAGER.code.length == 0,
                    "TipFeeManager should not exist on Ethereum"
                );
            }

            function test_fee_manager_callable_on_tempo() public view {
                require(
                    TIP_FEE_MANAGER.code.length > 0,
                    "TipFeeManager must be deployed on Tempo"
                );
            }
        }
        "#,
    );

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/inline.sol:ContractNetwork
[PASS] test_fee_manager_absent_on_ethereum() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/inline.sol:ContractNetwork
[PASS] test_fee_manager_callable_on_tempo() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
}

// TIP20 calls made in an inline Tempo pass should use the token name loaded from Tempo state in
// rendered traces.
#[forgetest]
fn tempo_pass_traces_tip20_name(prj: _, cmd: _) {
    prj.add_test(
        "inline.sol",
        r#"
        interface ITIP20 {
            function balanceOf(address account) external view returns (uint256);
        }

        contract TempoTraceLabels {
            address constant ALPHA_USD = 0x20C0000000000000000000000000000000000001;

            /// forge-config: default.networks.network = "tempo"
            function test_tip20_trace_uses_name() public view {
                ITIP20(ALPHA_USD).balanceOf(address(this));
            }
        }
        "#,
    );

    cmd.args(["test", "--mt", "test_tip20_trace_uses_name", "-vvvv"]).assert_success().stdout_eq(
        str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/inline.sol:TempoTraceLabels
[PASS] test_tip20_trace_uses_name() ([GAS])
Traces:
  [..] TempoTraceLabels::test_tip20_trace_uses_name()
    ├─ [..] AlphaUSD::balanceOf([..]) [staticcall]
    │   └─ ← [Return] [..]
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]],
    );
}

// Suites with no selected tests in a network pass must be skipped before setup.
#[forgetest]
fn network_pass_skips_unselected_setup(prj: _, cmd: _) {
    prj.add_test(
        "inline.sol",
        r#"
        /// forge-config: default.networks.network = "tempo"
        /// forge-config: default.hardfork = "tempo:T3"
        contract TempoNetwork {
            bool ready;

            function setUp() public {
                ready = true;
            }

            function test_contract_tempo() public view {
                require(ready, "setup must run in the selected pass");
            }
        }

        /// forge-config: default.networks.network = "ethereum"
        contract EthereumNetwork {
            function test_ethereum() public {}
        }

        contract FunctionNetwork {
            bool ready;

            function setUp() public {
                require(
                    address(0xfeEC000000000000000000000000000000000000).code.length > 0,
                    "setup must only run on Tempo"
                );
                ready = true;
            }

            /// forge-config: default.networks.network = "tempo"
            function test_tempo() public view {
                require(ready, "setup must run in the selected pass");
            }
        }
        "#,
    );

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/inline.sol:[..]Network
[PASS] test_[..]() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/inline.sol:[..]Network
[PASS] test_[..]() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/inline.sol:[..]Network
[PASS] test_[..]() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 3 test suites [ELAPSED]: 3 tests passed, 0 failed, 0 skipped (3 total tests)

"#]]);
}

// Inline config must not move the project root, which anchors `fs_permissions` and the
// `foundry.toml` write guard.
#[forgetest]
fn inline_root_is_ignored(prj: _, cmd: _) {
    prj.update_config(|_| {});
    let foundry_toml = prj.root().join("foundry.toml");
    let original = std::fs::read_to_string(&foundry_toml).unwrap();
    prj.add_test(
        "inline.sol",
        &r#"
        interface Vm {
            function projectRoot() external view returns (string memory);
            function writeFile(string calldata path, string calldata data) external;
        }

        /// forge-config: default.root = "/"
        /// forge-config: default.fs_permissions = [{ access = "read-write", path = "." }]
        contract InlineRoot {
            Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

            function test_root_unchanged() public view {
                require(keccak256(bytes(vm.projectRoot())) != keccak256("/"), "root moved");
            }

            function test_write_foundry_toml() public {
                vm.writeFile("FOUNDRY_TOML", "ffi = true");
            }
        }
    "#
        .replace("FOUNDRY_TOML", &foundry_toml.to_string_lossy().replace('\\', "/")),
    );

    cmd.args(["test", "--mt", "test_root_unchanged"]).assert_success();
    cmd.forge_fuse().args(["test", "--mt", "test_write_foundry_toml"]).assert_failure().stdout_eq(
        str![[r#"
No files changed, compilation skipped

Ran 1 test for test/inline.sol:InlineRoot
[FAIL: vm.writeFile: access to `foundry.toml` is not allowed] test_write_foundry_toml() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/inline.sol:InlineRoot
[FAIL: vm.writeFile: access to `foundry.toml` is not allowed] test_write_foundry_toml() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]],
    );
    assert_eq!(std::fs::read_to_string(&foundry_toml).unwrap(), original);
}
