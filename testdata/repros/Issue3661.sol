// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3661
contract Issue3661Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);
    address sender;

    function setUp() public {
        sender = msg.sender;
    }

    function testRollFork() public {
        assert(sender == msg.sender);
    }
}
