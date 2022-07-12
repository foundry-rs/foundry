// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "forge-std/Test.sol";
import "./Cheats.sol";

contract CoinbaseTest is Test {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testCoinbase() public {
        cheats.coinbase(0xEA674fdDe714fd979de3EdF0F56AA9716B898ec8);
        assertEq(block.coinbase, 0xEA674fdDe714fd979de3EdF0F56AA9716B898ec8, "coinbase failed");
    }

    function testCoinbaseFuzzed(address who) public {
        cheats.coinbase(who);
        assertEq(block.coinbase, who, "coinbase failed");
    }
}
