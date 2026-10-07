// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract EthGetProofTest is Test {
    function testEthGetProofRevertsWithoutFork() public {
        vm._expectCheatcodeRevert("no active fork URL found");
        vm.eth_getProof(address(this), new bytes32[](0), block.number);
    }
}
