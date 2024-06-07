// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract TearDownHandler {
    uint256 public count;

    function inc() external {
        count += 1;
    }
}

contract InvariantTearDownTest is DSTest {
    TearDownHandler handler;

    function setUp() public {
        handler = new TearDownHandler();
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = handler.inc.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function tearDown() public {
        require(handler.count() < 10, "teardown failure");
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 11
    function invariant_tear_down_failure() public view {
        require(handler.count() < 20, "invariant tear down failure");
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 11
    function invariant_failure() public view {
        require(handler.count() < 9, "invariant failure");
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 5
    function invariant_success() public view {
        require(handler.count() < 11, "invariant should not fail");
    }
}
