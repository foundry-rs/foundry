// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract BankTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testBank() public {
        cheats.bank(0xEA674fdDe714fd979de3EdF0F56AA9716B898ec8);
        assertEq(block.coinbase, 0xEA674fdDe714fd979de3EdF0F56AA9716B898ec8, "bank failed");
    }

    function testBankFuzzed(address who) public {
        address pre = block.coinbase;
        cheats.bank(who);
        assertEq(block.coinbase, who, "bank failed");
    }
}
