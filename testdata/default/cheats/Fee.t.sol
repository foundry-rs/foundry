// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract FeeTest is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    function testFee() public {
        VM.fee(10);
        assertEq(block.basefee, 10, "fee failed");
    }

    function testFeeFuzzed(uint64 fee) public {
        VM.fee(fee);
        assertEq(block.basefee, fee, "fee failed");
    }
}
