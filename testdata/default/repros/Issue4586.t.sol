// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/4586
contract Issue4586Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    uint256 constant initialBlock = 16730733;

    InvariantHandler handler;

    function setUp() public {
        vm.createSelectFork("mainnet", initialBlock);
        handler = new InvariantHandler();
    }

    function test_rollForkHandlerContract() public {
        assertEq(block.number, initialBlock);
        handler.rollFork();
        assertEq(block.number, initialBlock + 1);
    }

    function test_rollForkTestContract() public {
        assertEq(block.number, initialBlock);
        vm.rollFork(block.number + 1);
        assertEq(block.number, initialBlock + 1);
    }
}

contract InvariantHandler {
    address constant HEVM_ADDRESS = address(bytes20(uint160(uint256(keccak256("hevm cheat code")))));
    Vm constant vm = Vm(HEVM_ADDRESS);

    uint256 public calledRollFork;

    function rollFork() public {
        vm.rollFork(block.number + 1);
        calledRollFork += 1;
    }
}
