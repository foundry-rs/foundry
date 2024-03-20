// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract CoinbaseTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testCoinbase() public {
        vm.coinbase(0xEA674fdDe714fd979de3EdF0F56AA9716B898ec8);
        assertEq(block.coinbase, 0xEA674fdDe714fd979de3EdF0F56AA9716B898ec8, "coinbase failed");
    }

    function testCoinbaseFuzzed(address who) public {
        vm.coinbase(who);
        assertEq(block.coinbase, who, "coinbase failed");
    }
}
