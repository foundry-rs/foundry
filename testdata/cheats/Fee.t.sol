// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract FeeTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testFee() public {
        vm.fee(10);
        assertEq(block.basefee, 10, "fee failed");
    }

    function testFeeFuzzed(uint256 fee) public {
        vm.fee(fee);
        assertEq(block.basefee, fee, "fee failed");
    }
}
