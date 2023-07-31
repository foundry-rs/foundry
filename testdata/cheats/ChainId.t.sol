// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract DealTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testChainId() public {
        uint256 newChainId = 99;
        vm.chainId(newChainId);
        assertEq(newChainId, block.chainid);
    }
}
