// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/2629
contract Issue2629Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSelectFork() public {
        address coinbase = 0x0193d941b50d91BE6567c7eE1C0Fe7AF498b4137;

        uint256 f1 = vm.createSelectFork("mainnet", 9);
        vm.selectFork(f1);

        assertEq(block.number, 9);
        assertEq(coinbase.balance, 11250000000000000000);

        uint256 f2 = vm.createFork("mainnet", 10);
        vm.selectFork(f2);

        assertEq(block.number, 10);
        assertEq(coinbase.balance, 16250000000000000000);
    }
}
