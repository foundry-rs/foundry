// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6759
contract Issue6759Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testCreateMulti() public {
        uint256 fork1 = vm.createFork("mainnet", 10);
        uint256 fork2 = vm.createFork("mainnet", 10);
        uint256 fork3 = vm.createFork("mainnet", 10);
        assert(fork1 != fork2);
        assert(fork1 != fork3);
        assert(fork2 != fork3);
    }
}
