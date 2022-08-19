// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/2629
contract Issue2629Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function testSelectFork() public {
        address coinbase = 0x0193d941b50d91BE6567c7eE1C0Fe7AF498b4137;

        uint256 f1 = vm.createSelectFork("rpcAlias", 9);
        vm.selectFork(f1);

        assertEq(block.number, 9);
        assertEq(coinbase.balance, 11250000000000000000);

        uint256 f2 = vm.createFork("rpcAlias", 10);
        vm.selectFork(f2);

        assertEq(block.number, 10);
        assertEq(coinbase.balance, 16250000000000000000);
    }
}
