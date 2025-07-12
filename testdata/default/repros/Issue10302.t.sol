// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract A {
    function foo() public pure returns (bool) {
        return true;
    }
}

contract Issue10302Test is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    function testDelegateFails() external {
        VM.createSelectFork("sepolia");
        A a = new A();
        VM.startPrank(0x0fe884546476dDd290eC46318785046ef68a0BA9, true);
        (bool success,) = address(a).delegatecall(abi.encodeWithSelector(A.foo.selector));
        VM.stopPrank();
        require(success, "Delegate call should succeed");
    }

    function testDelegatePassesWhenBalanceSetToZero() external {
        VM.createSelectFork("sepolia");
        A a = new A();
        VM.startPrank(0x0fe884546476dDd290eC46318785046ef68a0BA9, true);
        VM.deal(0x0fe884546476dDd290eC46318785046ef68a0BA9, 0 ether);
        (bool success,) = address(a).delegatecall(abi.encodeWithSelector(A.foo.selector));
        VM.stopPrank();
        require(success, "Delegate call should succeed");
    }

    function testDelegateCallSucceeds() external {
        VM.createSelectFork("sepolia");
        A a = new A();
        VM.startPrank(0xd363339eE47775888Df411A163c586a8BdEA9dbf, true);
        (bool success,) = address(a).delegatecall(abi.encodeWithSelector(A.foo.selector));
        VM.stopPrank();
        require(success, "Delegate call should succeed");
    }

    function testDelegateFailsWhenBalanceGtZero() external {
        VM.createSelectFork("sepolia");
        A a = new A();
        VM.startPrank(0xd363339eE47775888Df411A163c586a8BdEA9dbf, true);
        VM.deal(0xd363339eE47775888Df411A163c586a8BdEA9dbf, 1 ether);
        (bool success,) = address(a).delegatecall(abi.encodeWithSelector(A.foo.selector));
        VM.stopPrank();
        require(success, "Delegate call should succeed");
    }
}
