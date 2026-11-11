// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {Test} from "forge-std/Test.sol";

contract MonadStorageProbe {
    function read(uint256 slot) external view returns (uint256 gasUsed, uint256 value) {
        assembly {
            let before := gas()
            value := sload(slot)
            gasUsed := sub(before, gas())
        }
    }

    function write(uint256 slotA, uint256 slotB, uint256 valueA, uint256 valueB)
        external
        returns (uint256 first, uint256 second)
    {
        assembly {
            let before := gas()
            sstore(slotA, valueA)
            let afterFirst := gas()
            sstore(slotB, valueB)
            let afterSecond := gas()
            first := sub(before, afterFirst)
            second := sub(afterFirst, afterSecond)
        }
    }
}

contract MonadStorageWarmthTest is Test {
    function test_store_seeds_warm_pages() public {
        MonadStorageProbe adjacent = new MonadStorageProbe();
        MonadStorageProbe separate = new MonadStorageProbe();
        vm.store(address(adjacent), bytes32(0), bytes32(uint256(11)));
        vm.store(address(adjacent), bytes32(uint256(1)), bytes32(uint256(22)));
        vm.store(address(separate), bytes32(0), bytes32(uint256(11)));
        vm.store(address(separate), bytes32(uint256(128)), bytes32(uint256(22)));

        (uint256 adjacentFirst, uint256 adjacentSecond) = adjacent.write(0, 1, 111, 222);
        (uint256 separateFirst, uint256 separateSecond) = separate.write(0, 128, 111, 222);
        assertEq(adjacentFirst, separateFirst);
        assertEq(separateSecond - adjacentSecond, 2800);
    }

    function test_store_preserves_first_write_charge() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        vm.store(address(probe), bytes32(0), bytes32(uint256(11)));
        (uint256 first, uint256 second) = probe.write(0, 0, 111, 222);
        assertEq(first - second, 2800);
    }

    function test_load_warms_only_its_page() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        assertEq(vm.load(address(probe), bytes32(0)), bytes32(0));
        (uint256 warm,) = probe.read(127);
        (uint256 cold,) = probe.read(128);
        assertEq(cold - warm, 8000);

        MonadStorageProbe other = new MonadStorageProbe();
        (uint256 otherCold,) = other.read(127);
        assertEq(otherCold, cold);
    }

    function test_explicit_slot_warmth_updates_page() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        vm.load(address(probe), bytes32(0));
        (uint256 warm,) = probe.read(127);
        vm.coolSlot(address(probe), bytes32(0));
        (uint256 cold,) = probe.read(127);
        assertEq(cold - warm, 8000);

        vm.coolSlot(address(probe), bytes32(uint256(127)));
        vm.warmSlot(address(probe), bytes32(0));
        (uint256 rewarmed,) = probe.read(127);
        assertEq(rewarmed, warm);
    }

    function test_cool_clears_loaded_pages() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        vm.load(address(probe), bytes32(0));
        vm.load(address(probe), bytes32(uint256(128)));
        (uint256 warm,) = probe.read(127);
        vm.cool(address(probe));
        (uint256 first,) = probe.read(127);
        (uint256 second,) = probe.read(255);
        assertEq(first - warm, 8000);
        assertEq(second, first);
    }

    function test_warmth_preserves_write_and_growth_accounting() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        (uint256 first, uint256 dirty) = probe.write(0, 0, 1, 2);
        assertEq(first - dirty, 8000 + 2800 + 17000);
        vm.coolSlot(address(probe), bytes32(0));
        vm.warmSlot(address(probe), bytes32(0));
        (uint256 clear, uint256 restore) = probe.write(0, 0, 0, 3);
        assertEq(clear, dirty);
        assertEq(restore, dirty);
    }

    function test_store_warmth_reverts_with_frame() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        try this.seedAndRevert(probe) {
            fail("expected revert");
        } catch {}
        (uint256 cold, uint256 value) = probe.read(0);
        assertEq(value, 0);
        (uint256 warm,) = probe.read(127);
        assertEq(cold - warm, 8000);
    }

    function seedAndRevert(MonadStorageProbe probe) external {
        vm.store(address(probe), bytes32(0), bytes32(uint256(11)));
        revert();
    }

    function test_snapshot_restores_page_warmth() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        uint256 snapshot = vm.snapshotState();
        vm.store(address(probe), bytes32(0), bytes32(uint256(11)));
        assertTrue(vm.revertToState(snapshot));
        (uint256 cold, uint256 value) = probe.read(0);
        assertEq(value, 0);

        snapshot = vm.snapshotState();
        vm.coolSlot(address(probe), bytes32(0));
        assertTrue(vm.revertToState(snapshot));
        (uint256 warm,) = probe.read(127);
        assertEq(cold - warm, 8000);
    }
}

contract StorageSlotWarmthTest is Test {
    function test_store_retains_slot_warmth_before_monad_ten() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        vm.store(address(probe), bytes32(0), bytes32(uint256(11)));
        (uint256 warm,) = probe.read(0);
        (uint256 adjacent,) = probe.read(1);
        (uint256 separate,) = probe.read(128);
        assertGt(adjacent, warm);
        assertEq(adjacent, separate);
    }
}
