// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6293
contract Issue6293Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    constructor() {
        require(address(this).balance > 0);
        payable(address(1)).call{value: 1}("");
    }

    function test() public {
        assertGt(address(this).balance, 0);
    }
}
