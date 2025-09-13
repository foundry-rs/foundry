// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "../src/test.sol";
import "../src/Vm.sol";
import "../src/SimpleRevert.sol";
import "../src/NestedCalls.sol";
import "../src/DelegateCall.sol";
import "../src/StaticCall.sol";

contract BacktraceTest is DSTest {
    SimpleRevert simpleRevert;
    NestedCalls nestedCalls;
    DelegateTarget delegateTarget;
    DelegateCaller delegateCaller;
    StaticTarget staticTarget;
    StaticCaller staticCaller;

    function setUp() public {
        simpleRevert = new SimpleRevert();
        nestedCalls = new NestedCalls();
        delegateTarget = new DelegateTarget();
        delegateCaller = new DelegateCaller(address(delegateTarget));
        staticTarget = new StaticTarget();
        staticCaller = new StaticCaller(address(staticTarget));
    }

    // Simple revert test
    function testSimpleRevert() public {
        simpleRevert.doRevert("Simple revert message");
    }

    // Require failure test
    function testRequireFail() public {
        simpleRevert.doRequire(0);
    }

    // Assert failure test
    function testAssertFail() public {
        simpleRevert.doAssert();
    }

    // Custom error test
    function testCustomError() public {
        simpleRevert.doCustomError();
    }

    // Nested calls test
    function testNestedCalls() public {
        nestedCalls.nestedCall(5);
    }

    // Internal call chain test
    function testInternalCallsSameSource() public {
        nestedCalls.callChain1();
    }

    // Test internal calls within test contract
    function testInternalCallChain() public {
        internalCall1();
    }

    function internalCall1() internal {
        internalCall2();
    }

    function internalCall2() internal {
        internalCall3();
    }

    function internalCall3() internal pure {
        revert("Failed at internal level 3");
    }

    // Delegate call revert test
    function testDelegateCallRevert() public {
        delegateCaller.delegateFail();
    }

    // Delegate call require test
    function testDelegateCallRequire() public {
        delegateCaller.delegateCompute(0, 5);
    }

    // Static call revert test
    function testStaticCallRevert() public view {
        staticCaller.staticCallFail();
    }

    // Static call require test
    function testStaticCallRequire() public view {
        staticCaller.staticCompute(0);
    }
}
