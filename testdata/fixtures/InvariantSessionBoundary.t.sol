// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract Handler {
    uint256 public count;
    bool public poisoned;

    function step() external {
        require(!poisoned, "predicate write leaked");
        require(count < 4, "run baseline leaked");
        count++;
    }

    function poison() external {
        poisoned = true;
    }
}

contract SessionTest {
    struct FuzzSelector {
        address addr;
        bytes4[] selectors;
    }
    Handler handler;

    function setUp() public {
        handler = new Handler();
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(handler);
    }

    function targetSelectors() public view returns (FuzzSelector[] memory targets) {
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = handler.step.selector;
        targets = new FuzzSelector[](1);
        targets[0] = FuzzSelector(address(handler), selectors);
    }

    function invariant_state() public {
        handler.poison();
    }

    function afterInvariant() public view {
        require(handler.count() == 4, "handler state not retained");
        require(!handler.poisoned(), "predicate write leaked at end");
    }
}
