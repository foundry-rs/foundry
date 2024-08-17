// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract NonPersistentHelper is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 public curState;

    function createSelectFork() external {
        vm.createSelectFork("mainnet");
        curState += 1;
    }

    function createSelectForkAtBlock() external {
        vm.createSelectFork("mainnet", 19000000);
        curState += 1;
    }

    function createSelectForkAtTx() external {
        vm.createSelectFork(
            "mainnet", vm.parseBytes32("0xb5c978f15d01fcc9b4d78967e8189e35ecc21ff4e78315ea5d616f3467003c84")
        );
        curState += 1;
    }

    function selectFork(uint256 forkId) external {
        vm.selectFork(forkId);
        curState += 1;
    }

    function rollForkAtBlock() external {
        vm.rollFork(19000000);
        curState += 1;
    }

    function rollForkIdAtBlock(uint256 forkId) external {
        vm.rollFork(forkId, 19000000);
        curState += 1;
    }
}

// https://github.com/foundry-rs/foundry/issues/8004
contract Issue8004CreateSelectForkTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    NonPersistentHelper helper;

    function setUp() public {
        helper = new NonPersistentHelper();
    }

    function testNonPersistentHelperCreateFork() external {
        helper.createSelectFork();
        assertEq(helper.curState(), 1);
    }

    function testNonPersistentHelperCreateForkAtBlock() external {
        helper.createSelectForkAtBlock();
        assertEq(helper.curState(), 1);
    }

    function testNonPersistentHelperCreateForkAtTx() external {
        helper.createSelectForkAtBlock();
        assertEq(helper.curState(), 1);
    }

    function testNonPersistentHelperSelectFork() external {
        uint256 forkId = vm.createFork("mainnet");
        helper.selectFork(forkId);
        assertEq(helper.curState(), 1);
    }
}

contract Issue8004RollForkTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    NonPersistentHelper helper;
    uint256 forkId;

    function setUp() public {
        forkId = vm.createSelectFork("mainnet");
        helper = new NonPersistentHelper();
    }

    function testNonPersistentHelperRollForkAtBlock() external {
        helper.rollForkAtBlock();
        assertEq(helper.curState(), 1);
    }

    function testNonPersistentHelperRollForkIdAtBlock() external {
        helper.rollForkIdAtBlock(forkId);
        assertEq(helper.curState(), 1);
    }
}
