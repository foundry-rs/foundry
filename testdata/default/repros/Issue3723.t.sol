// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3723
contract Issue3723Test is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    function testFailExample() public {
        VM.expectRevert();
        revert();

        VM.expectRevert();
        emit log_string("Do not revert");
    }
}
