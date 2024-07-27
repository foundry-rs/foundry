// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";
import "cheats/Vm.sol";

interface IERC20 {
    function totalSupply() external view returns (uint256 supply);
}

contract RollForkHandler is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 public totalSupply;

    function work() external {
        vm.rollFork(block.number + 1);
        totalSupply = IERC20(0x6B175474E89094C44Da98b954EedeAC495271d0F).totalSupply();
    }
}

contract InvariantRollForkBlockTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    RollForkHandler forkHandler;

    function setUp() public {
        vm.createSelectFork("mainnet", 19812632);
        forkHandler = new RollForkHandler();
    }

    /// forge-config: default.invariant.runs = 2
    /// forge-config: default.invariant.depth = 4
    function invariant_fork_handler_block() public {
        require(block.number < 19812634, "too many blocks mined");
    }
}

contract InvariantRollForkStateTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    RollForkHandler forkHandler;

    function setUp() public {
        vm.createSelectFork("mainnet", 19812632);
        forkHandler = new RollForkHandler();
    }

    /// forge-config: default.invariant.runs = 1
    function invariant_fork_handler_state() public {
        require(forkHandler.totalSupply() < 3254378807384273078310283461, "wrong supply");
    }
}
