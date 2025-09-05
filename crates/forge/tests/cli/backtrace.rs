//! Tests for backtrace functionality

forgetest!(test_backtraces, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.add_source("SimpleRevert.sol", include_str!("../fixtures/backtraces/SimpleRevert.sol"));
    prj.add_source("StaticCall.sol", include_str!("../fixtures/backtraces/StaticCall.sol"));
    prj.add_source("DelegateCall.sol", include_str!("../fixtures/backtraces/DelegateCall.sol"));
    prj.add_source("NestedCalls.sol", include_str!("../fixtures/backtraces/NestedCalls.sol"));

    prj.add_test("Backtrace.t.sol", include_str!("../fixtures/backtraces/Backtrace.t.sol"));

    let output = cmd.args(["test", "-vvv"]).assert_failure();

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
  at DelegateCaller.delegateCompute (src/DelegateCall.sol:33:20)
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
  at StaticCaller.staticCompute (src/StaticCall.sol:32:20)
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

    // Add initial source files
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

    // Add another source file that won't be modified (to test caching)
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

    // Add test file
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

    let output = cmd.args(["test", "-vvv"]).assert_failure();

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
    let output = cmd.forge_fuse().args(["test", "-vvv"]).assert_failure();

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
