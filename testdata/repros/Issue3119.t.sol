// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3119
contract Issue3119Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    address public owner = vm.addr(1);
    address public alice = vm.addr(2);

    function testRollFork() public {
        uint256 fork = vm.createFork("rpcAlias");
        vm.selectFork(fork);

        FortressSwap fortressSwap = new FortressSwap(address(owner));
        vm.prank(owner);
        fortressSwap.updateOwner(alice);
    }
}

contract FortressSwap {
    address owner;

    constructor(address _owner) {
        owner = _owner;
    }

    function updateOwner(address new_owner) public {
        require(msg.sender == owner, "must be owner");
        owner = new_owner;
    }
}
