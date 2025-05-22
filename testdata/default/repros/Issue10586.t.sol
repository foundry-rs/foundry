// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Target is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    event ChainId(uint256 indexed chainId);

    function setChainId() public {
        vm.chainId(123);
    }
}

contract Issue10586Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    Target public target;

    function setUp() public {
        target = new Target();
    }

    function testGetChainIdAfterSet() public {
        // By default, the chainId is 31337 during testing.
        assertEq(block.chainid, 31337);

        // Call external function to set the chainId to 123.
        target.setChainId();

        // The chainId is set to 123 in the block.
        assertEq(block.chainid, 123);

        // Set the chainId to 100.
        vm.chainId(100);

        // The chainId is set to 100 in the block.
        assertEq(block.chainid, 100);

        // Call the external function again, which will set the chainId to 123.
        target.setChainId();

        // The last call to chainId() will be the one that is set
        // in the block, so it should be 123.
        assertEq(block.chainid, 123);
    }
}
