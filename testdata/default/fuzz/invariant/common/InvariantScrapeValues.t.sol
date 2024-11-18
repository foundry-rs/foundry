// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract FindFromReturnValue {
    bool public found = false;

    function seed() public returns (int256) {
        int256 mystery = 13337;
        return (1337 + mystery);
    }

    function find(int256 i) public {
        int256 mystery = 13337;
        if (i == 1337 + mystery) {
            found = true;
        }
    }
}

contract FindFromReturnValueTest is DSTest {
    FindFromReturnValue target;

    function setUp() public {
        target = new FindFromReturnValue();
    }

    /// forge-config: default.invariant.runs = 50
    /// forge-config: default.invariant.depth = 300
    /// forge-config: default.invariant.fail-on-revert = true
    function invariant_value_not_found() public view {
        require(!target.found(), "value from return found");
    }
}

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
    /// forge-config: default.invariant.fail-on-revert = true
    function invariant_value_not_found() public view {
        require(!target.found(), "value from logs found");
    }
}
