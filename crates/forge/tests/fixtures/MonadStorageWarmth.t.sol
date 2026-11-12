// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {Test} from "forge-std/Test.sol";

interface VmIsolation {
    function isIsolateMode() external view returns (bool);
}

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
    // Each check runs inside one call so isolation starts one transaction for all its probes.
    function test_store_seeds_warm_pages() public {
        this.check_store_seeds_warm_pages();
    }

    function test_store_preserves_first_write_charge() public {
        this.check_store_preserves_first_write_charge();
    }

    function test_load_warms_only_its_page() public {
        this.check_load_warms_only_its_page();
    }

    function test_explicit_slot_warmth_updates_page() public {
        this.check_explicit_slot_warmth_updates_page();
    }

    function test_cool_clears_loaded_pages() public {
        this.check_cool_clears_loaded_pages();
    }

    function test_warmth_preserves_write_and_growth_accounting() public {
        this.check_warmth_preserves_write_and_growth_accounting();
    }

    function test_store_warmth_reverts_with_frame() public {
        this.check_store_warmth_reverts_with_frame();
    }

    function test_snapshot_restores_page_warmth() public {
        this.check_snapshot_restores_page_warmth();
    }

    function test_copy_storage_retains_native_page_warmth() public {
        this.check_copy_storage_retains_native_page_warmth();
    }

    function test_arbitrary_storage_retains_page_warmth() public {
        this.check_arbitrary_storage_retains_page_warmth();
    }

    function check_store_seeds_warm_pages() external {
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

    function check_store_preserves_first_write_charge() external {
        MonadStorageProbe probe = new MonadStorageProbe();
        vm.store(address(probe), bytes32(0), bytes32(uint256(11)));
        (uint256 first, uint256 second) = probe.write(0, 0, 111, 222);
        assertEq(first - second, 2800);
    }

    function check_load_warms_only_its_page() external {
        MonadStorageProbe probe = new MonadStorageProbe();
        assertEq(vm.load(address(probe), bytes32(0)), bytes32(0));
        (uint256 warm,) = probe.read(127);
        (uint256 cold,) = probe.read(128);
        assertEq(cold - warm, 8000);

        MonadStorageProbe other = new MonadStorageProbe();
        (uint256 otherCold,) = other.read(127);
        assertEq(otherCold, cold);
    }

    function check_explicit_slot_warmth_updates_page() external {
        MonadStorageProbe probe = new MonadStorageProbe();
        vm.load(address(probe), bytes32(0));
        (uint256 warm,) = probe.read(127);
        vm.coolSlot(address(probe), bytes32(0));
        (uint256 stillWarm,) = probe.read(127);
        assertEq(stillWarm, warm);
        vm.coolSlot(address(probe), bytes32(uint256(127)));
        (uint256 cold,) = probe.read(127);
        assertEq(cold - warm, 8000);

        vm.coolSlot(address(probe), bytes32(uint256(127)));
        vm.warmSlot(address(probe), bytes32(0));
        (uint256 rewarmed,) = probe.read(127);
        assertEq(rewarmed, warm);
    }

    function check_cool_clears_loaded_pages() external {
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

    function check_warmth_preserves_write_and_growth_accounting() external {
        MonadStorageProbe probe = new MonadStorageProbe();
        (uint256 first, uint256 dirty) = probe.write(0, 0, 1, 2);
        assertEq(first - dirty, 8000 + 2800 + 17000);
        vm.coolSlot(address(probe), bytes32(0));
        vm.warmSlot(address(probe), bytes32(0));
        (uint256 clear, uint256 restore) = probe.write(0, 0, 0, 3);
        assertEq(clear, dirty);
        assertEq(restore, dirty);
    }

    function check_store_warmth_reverts_with_frame() external {
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

    function check_snapshot_restores_page_warmth() external {
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
    function check_copy_storage_retains_native_page_warmth() external {
        MonadStorageProbe source = new MonadStorageProbe();
        MonadStorageProbe target = new MonadStorageProbe();
        vm.store(address(source), bytes32(0), bytes32(uint256(11)));
        vm.copyStorage(address(source), address(target));
        (uint256 warm,) = target.read(127);
        (uint256 cold,) = target.read(128);
        assertEq(cold - warm, 8000);
        (, uint256 value) = target.read(0);
        assertEq(value, 11);
    }

    function check_arbitrary_storage_retains_page_warmth() external {
        MonadStorageProbe source = new MonadStorageProbe();
        MonadStorageProbe target = new MonadStorageProbe();
        vm.setArbitraryStorage(address(source));
        vm.copyStorage(address(source), address(target));
        (uint256 cold, uint256 copied) = target.read(0);
        (uint256 warm,) = target.read(127);
        assertEq(cold - warm, 8000);
        (, uint256 original) = source.read(0);
        assertEq(original, copied);
    }

    function test_isolation_resets_native_page_warmth() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        (uint256 cold,) = probe.read(0);
        vm.store(address(probe), bytes32(0), bytes32(uint256(11)));
        (uint256 gasUsed, uint256 value) = probe.read(0);
        assertEq(value, 11);
        assertEq(cold - gasUsed, VmIsolation(address(vm)).isIsolateMode() ? 0 : 8000);
    }

    function test_isolation_resets_page_write_charges() public {
        MonadStorageProbe probe = new MonadStorageProbe();
        probe.write(0, 0, 1, 2);
        (uint256 first, uint256 second) = probe.write(0, 0, 3, 4);
        assertEq(first - second, VmIsolation(address(vm)).isIsolateMode() ? 10800 : 0);
    }
}

contract StorageSlotWarmthTest is Test {
    function test_store_retains_slot_warmth_before_monad_ten() public {
        this.checkSlotWarmth();
    }

    function checkSlotWarmth() external {
        MonadStorageProbe probe = new MonadStorageProbe();
        vm.store(address(probe), bytes32(0), bytes32(uint256(11)));
        (uint256 warm,) = probe.read(0);
        (uint256 adjacent,) = probe.read(1);
        (uint256 separate,) = probe.read(128);
        assertGt(adjacent, warm);
        assertEq(adjacent, separate);
    }
}


contract MonadForkStorageWarmthTest is Test {
    function test_fork_switch_retains_persistent_page_warmth() public {
        this.checkForkSwitch();
    }

    function checkForkSwitch() external {
        MonadStorageProbe probe = new MonadStorageProbe();
        vm.makePersistent(address(probe));
        string memory endpoint = vm.envString("MONAD_WARMTH_RPC");
        uint256 first = vm.createFork(endpoint);
        uint256 second = vm.createFork(endpoint);
        vm.selectFork(first);
        vm.store(address(probe), bytes32(0), bytes32(uint256(11)));
        (uint256 warm,) = probe.read(127);
        vm.selectFork(second);
        (uint256 afterSwitch,) = probe.read(126);
        assertEq(afterSwitch, warm);
        vm.selectFork(first);
        (uint256 afterReturn,) = probe.read(125);
        assertEq(afterReturn, warm);
        (, uint256 value) = probe.read(0);
        assertEq(value, 11);
        vm.cool(address(probe));
        (uint256 cold,) = probe.read(127);
        assertEq(cold - warm, 8000);
    }
}
