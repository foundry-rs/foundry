// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";

contract InvariantSenders {
    function checkSender() external {
        require(msg.sender != 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, "sender cannot be cheatcode address");
        require(msg.sender != 0x000000000000000000636F6e736F6c652e6c6f67, "sender cannot be console address");
        require(msg.sender != 0x4e59b44847b379578588920cA78FbF26c0B4956C, "sender cannot be CREATE2 deployer");
    }
}

contract InvariantExcludedSendersTest is DSTest {
    InvariantSenders target;

    function setUp() public {
        target = new InvariantSenders();
    }

    function invariant_check_sender() public view {}
}
