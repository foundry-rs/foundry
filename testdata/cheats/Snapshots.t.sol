// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

struct Storage {
    uint256 slot0;
    uint256 slot1;
}

contract SnapshotTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    Storage store;

    function setUp() public {
        store.slot0 = 10;
        store.slot1 = 20;
    }

    function testSnapshot() public {
        uint256 snapshot = vm.snapshot();
        store.slot0 = 300;
        store.slot1 = 400;

        assertEq(store.slot0, 300);
        assertEq(store.slot1, 400);

        vm.revertTo(snapshot);
        assertEq(store.slot0, 10, "snapshot revert for slot 0 unsuccessful");
        assertEq(store.slot1, 20, "snapshot revert for slot 1 unsuccessful");
    }

    // tests that snapshots can also revert changes to `block`
    function testBlockValues() public {
        uint256 num = block.number;
        uint256 time = block.timestamp;
        uint256 prevrandao = block.prevrandao;

        uint256 snapshot = vm.snapshot();

        vm.warp(1337);
        assertEq(block.timestamp, 1337);

        vm.roll(99);
        assertEq(block.number, 99);

        vm.prevrandao(bytes32(uint256(123)));
        assertEq(block.prevrandao, 123);

        assert(vm.revertTo(snapshot));

        assertEq(block.number, num, "snapshot revert for block.number unsuccessful");
        assertEq(block.timestamp, time, "snapshot revert for block.timestamp unsuccessful");
        assertEq(block.prevrandao, prevrandao, "snapshot revert for block.prevrandao unsuccessful");
    }
}
