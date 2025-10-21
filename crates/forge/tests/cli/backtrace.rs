//! Tests for backtrace functionality

use foundry_test_utils::rpc::{next_etherscan_api_key, next_http_rpc_endpoint};

forgetest!(test_backtraces, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.add_source("SimpleRevert.sol", include_str!("../fixtures/backtraces/SimpleRevert.sol"));
    prj.add_source("StaticCall.sol", include_str!("../fixtures/backtraces/StaticCall.sol"));
    prj.add_source("DelegateCall.sol", include_str!("../fixtures/backtraces/DelegateCall.sol"));
    prj.add_source("NestedCalls.sol", include_str!("../fixtures/backtraces/NestedCalls.sol"));

    prj.add_test("Backtrace.t.sol", include_str!("../fixtures/backtraces/Backtrace.t.sol"));

    let output = cmd.args(["test", "-vvvvv"]).assert_failure();

    output.stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
...
Ran 11 tests for test/Backtrace.t.sol:BacktraceTest
[FAIL: panic: assertion failed (0x01)] testAssertFail() ([GAS])
...
Backtrace:
  at SimpleRevert.doAssert
  at BacktraceTest.testAssertFail (test/Backtrace.t.sol:40:48)

[FAIL: CustomError(42, 0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496)] testCustomError() ([GAS])
...
Backtrace:
  at SimpleRevert.doCustomError (src/SimpleRevert.sol:21:59)
  at BacktraceTest.testCustomError (test/Backtrace.t.sol:45:49)

[FAIL: Delegate compute failed] testDelegateCallRequire() ([GAS])
...
Backtrace:
  at DelegateTarget.compute (src/DelegateCall.sol:11:84)
  at DelegateCaller.delegateCompute (src/DelegateCall.sol:32:101)
  at BacktraceTest.testDelegateCallRequire (test/Backtrace.t.sol:82:57)

[FAIL: Delegate call failed] testDelegateCallRevert() ([GAS])
...
Backtrace:
  at DelegateTarget.fail (src/DelegateCall.sol:7:43)
  at DelegateCaller.delegateFail (src/DelegateCall.sol:26:91)
  at BacktraceTest.testDelegateCallRevert (test/Backtrace.t.sol:77:56)

[FAIL: Failed at internal level 3] testInternalCallChain() ([GAS])
...
Backtrace:
  at BacktraceTest.testInternalCallChain (test/Backtrace.t.sol:72:54)

[FAIL: Failed at chain level 3] testInternalCallsSameSource() ([GAS])
...
Backtrace:
  at NestedCalls.callChain1 (src/NestedCalls.sol:25:51)
  at BacktraceTest.testInternalCallsSameSource (test/Backtrace.t.sol:55:61)

[FAIL: Maximum depth reached] testNestedCalls() ([GAS])
...
Backtrace:
  at NestedCalls.nestedCall (src/NestedCalls.sol:11:46)
  at NestedCalls.nestedCall (src/NestedCalls.sol:13:19)
  at NestedCalls.nestedCall (src/NestedCalls.sol:13:19)
  at NestedCalls.nestedCall (src/NestedCalls.sol:13:19)
  at NestedCalls.nestedCall (src/NestedCalls.sol:13:19)
  at BacktraceTest.testNestedCalls (test/Backtrace.t.sol:50:49)

[FAIL: Value must be greater than zero] testRequireFail() ([GAS])
...
Backtrace:
  at SimpleRevert.doRequire (src/SimpleRevert.sol:11:61)
  at BacktraceTest.testRequireFail (test/Backtrace.t.sol:35:49)

[FAIL: Simple revert message] testSimpleRevert() ([GAS])
...
Backtrace:
  at SimpleRevert.doRevert (src/SimpleRevert.sol:7:67)
  at BacktraceTest.testSimpleRevert (test/Backtrace.t.sol:30:50)

[FAIL: Static compute failed] testStaticCallRequire() ([GAS])
...
Backtrace:
  at StaticTarget.compute (src/StaticCall.sol:11:77)
  at StaticCaller.staticCompute (src/StaticCall.sol:30:124)
  at BacktraceTest.testStaticCallRequire (test/Backtrace.t.sol:92:60)

[FAIL: Static call reverted] testStaticCallRevert() ([GAS])
...
Backtrace:
  at StaticTarget.viewFail (src/StaticCall.sol:7:47)
  at StaticCaller.staticCallFail (src/StaticCall.sol:25:93)
  at BacktraceTest.testStaticCallRevert (test/Backtrace.t.sol:87:59)

Suite result: FAILED. 0 passed; 11 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

forgetest!(test_backtrace_with_mixed_compilation, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();

    prj.add_source(
        "SimpleRevert.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract SimpleRevert {
    function doRevert(string memory reason) public pure {
        revert(reason);
    }
}
"#,
    );

    // Add another source file that won't be modified
    prj.add_source(
        "HelperContract.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract HelperContract {
    function getValue() public pure returns (uint256) {
        return 42;
    }
    
    function doRevert() public pure {
        revert("Helper revert");
    }
}
"#,
    );

    prj.add_test(
        "BacktraceTest.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "../src/test.sol";
import "../src/SimpleRevert.sol";
import "../src/HelperContract.sol";

contract BacktraceTest is DSTest {
    SimpleRevert simpleRevert;
    HelperContract helper;
    
    function setUp() public {
        simpleRevert = new SimpleRevert();
        helper = new HelperContract();
    }
    
    function testSimpleRevert() public {
        simpleRevert.doRevert("Test failure");
    }
    
    function testHelperRevert() public {
        helper.doRevert();
    }
}
"#,
    );

    let output = cmd.args(["test", "-vvvvv"]).assert_failure();

    output.stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
Ran 2 tests for test/BacktraceTest.t.sol:BacktraceTest
[FAIL: Helper revert] testHelperRevert() ([GAS])
...
Backtrace:
  at HelperContract.doRevert (src/HelperContract.sol:11:47)
  at BacktraceTest.testHelperRevert (test/BacktraceTest.t.sol:23:50)

[FAIL: Test failure] testSimpleRevert() ([GAS])
...
Backtrace:
  at SimpleRevert.doRevert (src/SimpleRevert.sol:7:67)
  at BacktraceTest.testSimpleRevert (test/BacktraceTest.t.sol:19:50)

Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]
...
"#]]);

    // Modify the source file - add a comment to change line numbers
    prj.add_source(
        "SimpleRevert.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract SimpleRevert {
    function doRevert(string memory reason) public pure {
        // Added comment to shift line numbers
        revert(reason);
    }
}
"#,
    );

    // Modify the test file as well
    prj.add_test(
        "BacktraceTest.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "../src/test.sol";
import "../src/SimpleRevert.sol";
import "../src/HelperContract.sol";

contract BacktraceTest is DSTest {
    SimpleRevert simpleRevert;
    HelperContract helper;
    
    function setUp() public {
        simpleRevert = new SimpleRevert();
        helper = new HelperContract();
    }
    
    function testSimpleRevert() public {
        // Added some comments
        // to change line numbers
        simpleRevert.doRevert("Test failure");
    }
    
    function testHelperRevert() public {
        helper.doRevert();
    }
}
"#,
    );

    // Second run - mixed compilation (SimpleRevert fresh, BacktraceTest fresh, HelperContract
    // cached)
    let output = cmd.forge_fuse().args(["test", "-vvvvv"]).assert_failure();

    output.stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
Ran 2 tests for test/BacktraceTest.t.sol:BacktraceTest
[FAIL: Helper revert] testHelperRevert() ([GAS])
...
Backtrace:
  at HelperContract.doRevert (src/HelperContract.sol:11:47)
  at BacktraceTest.testHelperRevert (test/BacktraceTest.t.sol:25:50)

[FAIL: Test failure] testSimpleRevert() ([GAS])
...
Backtrace:
  at SimpleRevert.doRevert (src/SimpleRevert.sol:8:56)
  at BacktraceTest.testSimpleRevert (test/BacktraceTest.t.sol:21:43)

Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

forgetest!(test_library_backtrace, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();

    // Add library source files
    prj.add_source(
        "libraries/InternalMathLib.sol",
        include_str!("../fixtures/backtraces/libraries/InternalMathLib.sol"),
    );
    prj.add_source(
        "libraries/ExternalMathLib.sol",
        include_str!("../fixtures/backtraces/libraries/ExternalMathLib.sol"),
    );
    prj.add_source(
        "LibraryConsumer.sol",
        include_str!("../fixtures/backtraces/LibraryConsumer.sol"),
    );

    // Add test file
    prj.add_test(
        "LibraryBacktrace.t.sol",
        include_str!("../fixtures/backtraces/LibraryBacktrace.t.sol"),
    );

    // Add foundry.toml configuration for linked library
    let config = foundry_config::Config {
        libraries: vec!["src/libraries/ExternalMathLib.sol:ExternalMathLib:0x1234567890123456789012345678901234567890".to_string()],
        ..Default::default()
    };
    prj.write_config(config);

    let output =
        cmd.args(["test", "-vvv", "--ast", "--mc", "LibraryBacktraceTest"]).assert_failure();

    output.stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 9 tests for test/LibraryBacktrace.t.sol:LibraryBacktraceTest
[FAIL: DivisionByZero()] testExternalDivisionByZero() ([GAS])
...
Backtrace:
  at ExternalMathLib.div
  at LibraryConsumer.externalDivide
  at LibraryBacktraceTest.testExternalDivisionByZero

[FAIL: panic: arithmetic underflow or overflow (0x11)] testExternalOverflow() ([GAS])
...
Backtrace:
  at ExternalMathLib.mul
  at LibraryConsumer.externalMultiply
  at LibraryBacktraceTest.testExternalOverflow

[FAIL: ExternalMathLib: value must be positive] testExternalRequire() ([GAS])
...
Backtrace:
  at ExternalMathLib.requirePositive
  at LibraryConsumer.externalCheckPositive
  at LibraryBacktraceTest.testExternalRequire

[FAIL: Underflow()] testExternalUnderflow() ([GAS])
...
Backtrace:
  at ExternalMathLib.sub
  at LibraryConsumer.externalSubtract
  at LibraryBacktraceTest.testExternalUnderflow

[FAIL: DivisionByZero()] testInternalDivisionByZero() ([GAS])
...
Backtrace:
  at LibraryConsumer.internalDivide
  at LibraryBacktraceTest.testInternalDivisionByZero

[FAIL: panic: arithmetic underflow or overflow (0x11)] testInternalOverflow() ([GAS])
Traces:
...
Backtrace:
  at LibraryConsumer.internalMultiply
  at LibraryBacktraceTest.testInternalOverflow

[FAIL: InternalMathLib: value must be positive] testInternalRequire() ([GAS])
Traces:
...
Backtrace:
  at LibraryConsumer.internalCheckPositive
  at LibraryBacktraceTest.testInternalRequire

[FAIL: Underflow()] testInternalUnderflow() ([GAS])
Traces:
...
Backtrace:
  at LibraryConsumer.internalSubtract
  at LibraryBacktraceTest.testInternalUnderflow

[FAIL: DivisionByZero()] testMixedLibraryFailure() ([GAS])
Traces:
...
Backtrace:
  at ExternalMathLib.div
  at LibraryConsumer.mixedCalculation
  at LibraryBacktraceTest.testMixedLibraryFailure

Suite result: FAILED. 0 passed; 9 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

forgetest!(test_multiple_libraries_same_file, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "libraries/MultipleLibraries.sol",
        include_str!("../fixtures/backtraces/libraries/MultipleLibraries.sol"),
    );
    prj.add_source(
        "MultipleLibraryConsumer.sol",
        include_str!("../fixtures/backtraces/MultipleLibraryConsumer.sol"),
    );

    prj.add_test(
        "MultipleLibraryBacktrace.t.sol",
        include_str!("../fixtures/backtraces/MultipleLibraryBacktrace.t.sol"),
    );

    let output = cmd
        .args(["test", "-vvvvv", "--ast", "--mc", "MultipleLibraryBacktraceTest"])
        .assert_failure();

    output.stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 4 tests for test/MultipleLibraryBacktrace.t.sol:MultipleLibraryBacktraceTest
[FAIL: FirstLibError()] testAllLibrariesFirstFails() ([GAS])
...
Backtrace:
  at MultipleLibraryConsumer.useAllLibraries (src/libraries/MultipleLibraries.sol:10:42)
  at MultipleLibraryBacktraceTest.testAllLibrariesFirstFails (test/MultipleLibraryBacktrace.t.sol:31:60)

[FAIL: FirstLibError()] testFirstLibraryError() ([GAS])
Traces:
...
Backtrace:
  at MultipleLibraryConsumer.useFirstLib (src/libraries/MultipleLibraries.sol:10:42)
  at MultipleLibraryBacktraceTest.testFirstLibraryError (test/MultipleLibraryBacktrace.t.sol:16:55)

[FAIL: SecondLibError()] testSecondLibraryError() ([GAS])
Traces:
...
Backtrace:
  at MultipleLibraryConsumer.useSecondLib (src/libraries/MultipleLibraries.sol:26:41)
  at MultipleLibraryBacktraceTest.testSecondLibraryError (test/MultipleLibraryBacktrace.t.sol:21:56)

[FAIL: ThirdLibError()] testThirdLibraryError() ([GAS])
Traces:
...
Backtrace:
  at MultipleLibraryConsumer.useThirdLib (src/libraries/MultipleLibraries.sol:42:42)
  at MultipleLibraryBacktraceTest.testThirdLibraryError (test/MultipleLibraryBacktrace.t.sol:26:55)

Suite result: FAILED. 0 passed; 4 failed; 0 skipped; [ELAPSED]

...
"#]]);
});

forgetest!(test_fork_backtrace, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();

    let etherscan_api_key = next_etherscan_api_key();
    let fork_url = next_http_rpc_endpoint();

    prj.add_source(
        "ForkedERC20Wrapper.sol",
        include_str!("../fixtures/backtraces/ForkedERC20Wrapper.sol"),
    );

    prj.add_test("ForkBacktrace.t.sol", include_str!("../fixtures/backtraces/ForkBacktrace.t.sol"));

    let output = cmd
        .args(["test", "-vvvvv", "--fork-url", &fork_url, "--match-contract", "ForkBacktraceTest"])
        .assert_failure();

    output.stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
Ran 5 tests for test/ForkBacktrace.t.sol:ForkBacktraceTest
[FAIL: USDC transfer failed] testDirectOnChainRevert() ([GAS])
...
Backtrace:
  at 0x43506849D7C04F9138D1A2050bbF3A0c054402dd.transfer
  at 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48.transfer
  at ForkBacktraceTest.testDirectOnChainRevert (test/ForkBacktrace.t.sol:36:126)

[FAIL: ERC20: transfer amount exceeds balance] testNestedFailure() ([GAS])
...
Backtrace:
  at 0x43506849D7C04F9138D1A2050bbF3A0c054402dd.transfer
  at 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48.transfer
  at ForkedERC20Wrapper.nestedFailure (src/ForkedERC20Wrapper.sol:14:89)
  at ForkBacktraceTest.testNestedFailure (test/ForkBacktrace.t.sol:30:51)

[FAIL: Account has zero USDC balance] testRequireNonZeroBalance() ([GAS])
...
Backtrace:
  at ForkedERC20Wrapper.requireNonZeroBalance (src/ForkedERC20Wrapper.sol:23:68)
  at ForkBacktraceTest.testRequireNonZeroBalance (test/ForkBacktrace.t.sol:26:64)

[FAIL: ERC20: transfer amount exceeds allowance] testTransferFromWithoutApproval() ([GAS])
...
Backtrace:
  at 0x43506849D7C04F9138D1A2050bbF3A0c054402dd.transferFrom
  at 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48.transferFrom
  at ForkedERC20Wrapper.transferFromWithoutApproval (src/ForkedERC20Wrapper.sol:18:101)
  at ForkBacktraceTest.testTransferFromWithoutApproval (test/ForkBacktrace.t.sol:22:65)

[FAIL: ERC20: transfer amount exceeds balance] testTransferWithoutBalance() ([GAS])
...
Backtrace:
  at 0x43506849D7C04F9138D1A2050bbF3A0c054402dd.transfer
  at 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48.transfer
  at ForkedERC20Wrapper.transferWithoutBalance (src/ForkedERC20Wrapper.sol:14:89)
  at ForkBacktraceTest.testTransferWithoutBalance (test/ForkBacktrace.t.sol:18:60)

Suite result: FAILED. 0 passed; 5 failed; 0 skipped; [ELAPSED]
...
"#]]);

    cmd.forge_fuse()
        .args([
            "test",
            "--mt",
            "testTransferFromWithoutApproval",
            "-vvvvv",
            "--fork-url",
            &fork_url,
            "--etherscan-api-key",
            &etherscan_api_key,
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
No files changed, compilation skipped
...
Ran 1 test for test/ForkBacktrace.t.sol:ForkBacktraceTest
[FAIL: ERC20: transfer amount exceeds allowance] testTransferFromWithoutApproval() ([GAS])
...
Backtrace:
  at FiatTokenV2_2.transferFrom
  at FiatTokenProxy.fallback
  at ForkedERC20Wrapper.transferFromWithoutApproval (src/ForkedERC20Wrapper.sol:18:101)
  at ForkBacktraceTest.testTransferFromWithoutApproval (test/ForkBacktrace.t.sol:22:65)
...
"#]]);
});

forgetest!(test_backtrace_via_ir_disables_source_lines, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.add_source("SimpleRevert.sol", include_str!("../fixtures/backtraces/SimpleRevert.sol"));
    prj.add_source("StaticCall.sol", include_str!("../fixtures/backtraces/StaticCall.sol"));
    prj.add_source("DelegateCall.sol", include_str!("../fixtures/backtraces/DelegateCall.sol"));
    prj.add_source("NestedCalls.sol", include_str!("../fixtures/backtraces/NestedCalls.sol"));

    prj.add_test("Backtrace.t.sol", include_str!("../fixtures/backtraces/Backtrace.t.sol"));

    prj.update_config(|c| c.via_ir = true);

    let output = cmd.args(["test", "-vvvvv"]).assert_failure();
    output.stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
[FAIL: Static compute failed] testStaticCallRequire() ([GAS])
...
Backtrace:
  at StaticTarget.compute
  at StaticCaller.staticCompute
  at BacktraceTest.testStaticCallRequire
...
"#]]);
});
