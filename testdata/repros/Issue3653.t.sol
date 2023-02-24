// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3653
contract Issue3653Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);
    uint256 fork;
    Token token;

    constructor() {
        fork = vm.createSelectFork("rpcAlias", 10);
        token = new Token();
        vm.makePersistent(address(token));
    }

    function testDummy() public {
        assertEq(block.number, 10);
    }
}

contract Token {}
