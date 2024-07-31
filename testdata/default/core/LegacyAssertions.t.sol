// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract NoAssertionsRevertTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testMultipleAssertFailures() public {
        vm.assertEq(uint256(1), uint256(2));
        vm.assertLt(uint256(5), uint256(4));
    }
}

contract LegacyAssertionsTest {
    bool public failed;

    function testFlagNotSetSuccess() public {}

    function testFlagSetFailure() public {
        failed = true;
    }
}
