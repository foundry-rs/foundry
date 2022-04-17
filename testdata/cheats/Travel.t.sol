// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract ChainIdTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testChainId() public {
        cheats.chainId(10);
        assertEq(block.chainid, 10, "chainId switch failed");
    }

    function testChainIdFuzzed(uint128 jump) public {
        uint pre = block.chainid;
        cheats.chainId(block.chainid + jump);
        assertEq(block.chainid, pre + jump, "chainId switch failed");
    }
}
