// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

// https://github.com/foundry-rs/foundry/issues/7219

contract Handler is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function thisFunctionReverts() external {
        if (block.number < 10) {} else {
            revert();
        }
    }

    function advanceTime(uint256 blocks) external {
        blocks = blocks % 10;
        vm.roll(block.number + blocks);
        vm.warp(block.timestamp + blocks * 12);
    }
}

contract InvariantPreserveState is DSTest {
    Handler handler;

    function setUp() public {
        handler = new Handler();
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = handler.thisFunctionReverts.selector;
        selectors[1] = handler.advanceTime.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function invariant_preserve_state() public {
        assertTrue(true);
    }
}
