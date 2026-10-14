// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
import {VRFExternalSubOwnerExample} from "../src/tests/VRFExternalSubOwnerExample.sol";

contract CallbackTest {
    VRFExternalSubOwnerExample consumer;
    uint32 constant WORDS_REQUESTED = 3;

    function setUp() public {
        consumer = new VRFExternalSubOwnerExample(address(this), address(0));
        consumer.requestRandomWords(1, 100_000, 3, WORDS_REQUESTED, bytes32(0));
    }

    function requestRandomWords(bytes32, uint64, uint16, uint32 gasLimit, uint32) external pure returns (uint256) {
        require(gasLimit == 100_000);
        return 1;
    }

    function testReady() public {}
}
