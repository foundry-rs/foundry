// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

// https://github.com/foundry-rs/foundry/issues/3661
contract Issue3661Test is DSTest {
    address sender;

    function setUp() public {
        sender = msg.sender;
    }

    function testSameSender() public {
        assert(sender == msg.sender);
    }
}
