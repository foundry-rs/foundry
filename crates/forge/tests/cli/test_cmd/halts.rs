//! Snapshots of engine-derived halt reasons.

use foundry_test_utils::str;

#[forgetest_init]
fn halts_trace(prj: _, cmd: _) {
    prj.update_config(|config| config.code_size_limit = Some(24_576));
    prj.add_test(
        "Halts.t.sol",
        r#"
pragma solidity ^0.8.18;
import "forge-std/Test.sol";

contract HaltsTest is Test {
    function testHalts() public {
        vm.etch(address(0x1000), hex"fe");
        vm.etch(address(0x1001), hex"600056");
        vm.etch(address(0x1002), hex"0c");
        vm.etch(address(0x1003), hex"01");
        vm.etch(address(0x1004), hex"6001600055");
        vm.etch(address(0x1005), hex"5b600056");
        address(0x1000).call{gas: 50_000}("");
        address(0x1001).call{gas: 50_000}("");
        address(0x1002).call{gas: 50_000}("");
        address(0x1003).call{gas: 50_000}("");
        address(0x1004).staticcall{gas: 50_000}("");
        address(0x1005).call{gas: 50_000}("");
        // Return 24,577 bytes of runtime code.
        bytes memory initcode = hex"6160016000f3";
        assembly { pop(create(0, add(initcode, 32), mload(initcode))) }
    }
}
"#,
    );
    cmd.args(["build"]).assert_success();
    cmd.forge_fuse().args(["test", "-vvvv"]).assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/Halts.t.sol:HaltsTest
[PASS] testHalts() ([GAS])
Traces:
  [..] HaltsTest::testHalts()
    ├─ [..] VM::etch(0x0000000000000000000000000000000000001000, 0xfe)
    │   └─ ← [Return]
    ├─ [..] VM::etch(0x0000000000000000000000000000000000001001, 0x600056)
    │   └─ ← [Return]
    ├─ [..] VM::etch(0x0000000000000000000000000000000000001002, 0x0c)
    │   └─ ← [Return]
    ├─ [..] VM::etch(0x0000000000000000000000000000000000001003, 0x01)
    │   └─ ← [Return]
    ├─ [..] VM::etch(0x0000000000000000000000000000000000001004, 0x6001600055)
    │   └─ ← [Return]
    ├─ [..] VM::etch(0x0000000000000000000000000000000000001005, 0x5b600056)
    │   └─ ← [Return]
    ├─ [..] 0x0000000000000000000000000000000000001000::fallback()
    │   └─ ← [InvalidFEOpcode] EvmError: InvalidFEOpcode
    ├─ [..] 0x0000000000000000000000000000000000001001::fallback()
    │   └─ ← [InvalidJump] EvmError: InvalidJump
    ├─ [..] 0x0000000000000000000000000000000000001002::fallback()
    │   └─ ← [OpcodeNotFound] EvmError: OpcodeNotFound
    ├─ [..] 0x0000000000000000000000000000000000001003::fallback()
    │   └─ ← [StackUnderflow] EvmError: StackUnderflow
    ├─ [..] 0x0000000000000000000000000000000000001004::fallback() [staticcall]
    │   └─ ← [StateChangeDuringStaticCall] EvmError: StateChangeDuringStaticCall
    ├─ [..] 0x0000000000000000000000000000000000001005::fallback()
    │   └─ ← [OutOfGas] EvmError: OutOfGas
    ├─ [..] → new <unknown>@0xa0Cb889707d426A7A386870A03bc70d1b0697598
    │   └─ ← [CreateContractSizeLimit]
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
}

#[forgetest_init]
fn halts_evm_error(prj: _, cmd: _) {
    prj.update_config(|config| config.gas_limit = 1_000_000.into());
    prj.add_test(
        "HaltErrors.t.sol",
        r#"
pragma solidity ^0.8.18;
contract HaltErrorsTest {
    function testInvalid() public pure { assembly { invalid() } }
    function testOutOfGas() public pure { assembly { for {} 1 {} {} } }
}
"#,
    );
    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/HaltErrors.t.sol:HaltErrorsTest
[FAIL: EvmError: InvalidFEOpcode] testInvalid() ([GAS])
[FAIL: EvmError: OutOfGas] testOutOfGas() ([GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 2 failing tests in test/HaltErrors.t.sol:HaltErrorsTest
[FAIL: EvmError: InvalidFEOpcode] testInvalid() ([GAS])
[FAIL: EvmError: OutOfGas] testOutOfGas() ([GAS])

Encountered a total of 2 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 2 failed tests
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
}

#[forgetest_init]
fn halts_json(prj: _, cmd: _) {
    prj.add_test(
        "HaltJson.t.sol",
        r#"
pragma solidity ^0.8.18;
import "forge-std/Test.sol";
contract HaltJsonTest is Test {
    function testInvalidChild() public {
        vm.etch(address(0x1000), hex"fe");
        address(0x1000).call{gas: 50_000}("");
    }
}
"#,
    );
    cmd.args(["test", "--json", "-vvvv"]).assert_json_stdout(str![[r#"
{
  "test/HaltJson.t.sol:HaltJsonTest": {
    "duration": "{...}",
    "test_results": {
      "testInvalidChild()": {
        "status": "Success",
        "reason": null,
        "counterexample": null,
        "logs": [],
        "decoded_logs": [],
        "kind": {
          "Unit": {
            "gas": "{...}"
          }
        },
        "traces": [
          [
            "Deployment",
            {
              "arena": [
                {
                  "parent": null,
                  "children": [],
                  "idx": 0,
                  "trace": {
                    "depth": 0,
                    "success": true,
                    "caller": "0x1804c8ab1f12e6bbf3894d4083f33e07309d1f38",
                    "address": "0x7fa9385be102ac3eac297483dd6233d62b3e1496",
                    "maybe_precompile": false,
                    "selfdestruct_address": null,
                    "selfdestruct_refund_target": null,
                    "selfdestruct_transferred_value": null,
                    "kind": "CREATE",
                    "value": "0x0",
                    "data": "{...}",
                    "output": "{...}",
                    "bytecode": null,
                    "gas_used": "{...}",
                    "gas_limit": "{...}",
                    "gas_refund_counter": "{...}",
                    "status": "Return",
                    "steps": [],
                    "step_deltas": [],
                    "decoded": null
                  },
                  "logs": [],
                  "ordering": []
                }
              ]
            }
          ],
          [
            "Execution",
            {
              "arena": [
                {
                  "parent": null,
                  "children": [
                    1,
                    2
                  ],
                  "idx": 0,
                  "trace": {
                    "depth": 0,
                    "success": true,
                    "caller": "0x1804c8ab1f12e6bbf3894d4083f33e07309d1f38",
                    "address": "0x7fa9385be102ac3eac297483dd6233d62b3e1496",
                    "maybe_precompile": null,
                    "selfdestruct_address": null,
                    "selfdestruct_refund_target": null,
                    "selfdestruct_transferred_value": null,
                    "kind": "CALL",
                    "value": "0x0",
                    "data": "0xd8699570",
                    "output": "0x",
                    "bytecode": null,
                    "gas_used": "{...}",
                    "gas_limit": "{...}",
                    "gas_refund_counter": "{...}",
                    "status": "Stop",
                    "steps": [],
                    "step_deltas": [],
                    "decoded": null
                  },
                  "logs": [],
                  "ordering": [
                    {
                      "Call": 0
                    },
                    {
                      "Call": 1
                    }
                  ]
                },
                {
                  "parent": 0,
                  "children": [],
                  "idx": 1,
                  "trace": {
                    "depth": 1,
                    "success": true,
                    "caller": "0x7fa9385be102ac3eac297483dd6233d62b3e1496",
                    "address": "0x7109709ecfa91a80626ff3989d68f67f5b1dd12d",
                    "maybe_precompile": null,
                    "selfdestruct_address": null,
                    "selfdestruct_refund_target": null,
                    "selfdestruct_transferred_value": null,
                    "kind": "CALL",
                    "value": "0x0",
                    "data": "0xb4d6c782000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000001fe00000000000000000000000000000000000000000000000000000000000000",
                    "output": "0x",
                    "bytecode": null,
                    "gas_used": "{...}",
                    "gas_limit": "{...}",
                    "gas_refund_counter": "{...}",
                    "status": "Return",
                    "steps": [],
                    "step_deltas": [],
                    "decoded": null
                  },
                  "logs": [],
                  "ordering": []
                },
                {
                  "parent": 0,
                  "children": [],
                  "idx": 2,
                  "trace": {
                    "depth": 1,
                    "success": false,
                    "caller": "0x7fa9385be102ac3eac297483dd6233d62b3e1496",
                    "address": "0x0000000000000000000000000000000000001000",
                    "maybe_precompile": null,
                    "selfdestruct_address": null,
                    "selfdestruct_refund_target": null,
                    "selfdestruct_transferred_value": null,
                    "kind": "CALL",
                    "value": "0x0",
                    "data": "0x",
                    "output": "0x",
                    "bytecode": null,
                    "gas_used": "{...}",
                    "gas_limit": "{...}",
                    "gas_refund_counter": "{...}",
                    "status": "InvalidFEOpcode",
                    "steps": [],
                    "step_deltas": [],
                    "decoded": null
                  },
                  "logs": [],
                  "ordering": []
                }
              ]
            }
          ]
        ],
        "labeled_addresses": {},
        "duration": "{...}",
        "breakpoints": {},
        "gas_snapshots": {}
      }
    },
    "warnings": []
  }
}
"#]]);
}
