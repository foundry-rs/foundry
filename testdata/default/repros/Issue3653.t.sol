// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3653
contract Issue3653Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 fork;
    Token token;

    constructor() {
        fork = vm.createSelectFork("mainnet", 1000000);
        token = new Token();
        vm.makePersistent(address(token));
    }

    function testDummy() public {
        assertEq(block.number, 1000000);
    }
}

contract Token {}
