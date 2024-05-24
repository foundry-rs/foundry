// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RollForkHandler is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function work() external {
        vm.rollFork(block.number + 1);
    }
}

contract InvariantRollForkTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    RollForkHandler forkHandler;

    function setUp() public {
        vm.createSelectFork("rpcAlias", 19812632);
        forkHandler = new RollForkHandler();
    }

    /// forge-config: default.invariant.runs = 2
    /// forge-config: default.invariant.depth = 4
    function invariant_fork_handler_block() public {
        require(block.number < 19812634, "too many blocks mined");
    }
}
