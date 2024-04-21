// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract FindFromLogValue {
    event FindFromLog(int256 indexed mystery, bytes32 rand);
    bool public found = false;

    function seed() public {
        int256 mystery = 13337;
        emit FindFromLog(1337 + mystery, keccak256(abi.encodePacked("mystery")));
    }

    function find(int256 i) public {
        int256 mystery = 13337;
        if (i == 1337 + mystery) {
            found = true;
        }
    }
}

contract FindFromLogValueTest is DSTest {
    FindFromLogValue target;

    function setUp() public {
        target = new FindFromLogValue();
    }

    /// forge-config: default.invariant.runs = 50
    /// forge-config: default.invariant.depth = 300
    function invariant_value_not_found() public view {
        require(!target.found(), "value found");
    }
}
