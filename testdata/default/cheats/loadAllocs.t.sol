// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract LoadAllocsTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    address constant ALLOCD = address(0x420);
    address constant ALLOCD_B = address(0x421);

    uint256 snapshotId;
    string allocsPath;

    function setUp() public {
        allocsPath = string.concat(vm.projectRoot(), "/fixtures/Json/test_allocs.json");

        // Snapshot the state; We'll restore it in each test that loads allocs inline.
        snapshotId = vm.snapshotState();

        // Load the allocs file.
        vm.loadAllocs(allocsPath);
    }

    /// @dev Checks that the `loadAllocs` cheatcode persists account info if called in `setUp`
    function testLoadAllocsStaticSetup() public {
        // Balance should be `0xabcd`
        assertEq(ALLOCD.balance, 0xabcd);

        // Code should be a simple store / return, returning `0x42`
        (bool success, bytes memory rd) = ALLOCD.staticcall("");
        assertTrue(success);
        uint256 ret = abi.decode(rd, (uint256));
        assertEq(ret, 0x42);

        // Storage should have been set in slot 0x1, equal to `0xbeef`
        assertEq(uint256(vm.load(ALLOCD, bytes32(uint256(0x10 << 248)))), 0xbeef);
    }

    /// @dev Checks that the `loadAllocs` cheatcode persists account info if called inline
    function testLoadAllocsStatic() public {
        // Restore the state snapshot prior to the allocs file being loaded.
        vm.revertToState(snapshotId);

        // Load the allocs file
        vm.loadAllocs(allocsPath);

        // Balance should be `0xabcd`
        assertEq(ALLOCD.balance, 0xabcd);

        // Code should be a simple store / return, returning `0x42`
        (bool success, bytes memory rd) = ALLOCD.staticcall("");
        assertTrue(success);
        uint256 ret = abi.decode(rd, (uint256));
        assertEq(ret, 0x42);

        // Storage should have been set in slot 0x1, equal to `0xbeef`
        assertEq(uint256(vm.load(ALLOCD, bytes32(uint256(0x10 << 248)))), 0xbeef);
    }

    /// @dev Checks that the `loadAllocs` cheatcode overrides existing account information (if present)
    function testLoadAllocsOverride() public {
        // Restore the state snapshot prior to the allocs file being loaded.
        vm.revertToState(snapshotId);

        // Populate the alloc'd account's code.
        vm.etch(ALLOCD, hex"FF");
        assertEq(ALLOCD.code, hex"FF");

        // Store something in the alloc'd storage slot.
        bytes32 slot = bytes32(uint256(0x10 << 248));
        vm.store(ALLOCD, slot, bytes32(uint256(0xBADC0DE)));
        assertEq(uint256(vm.load(ALLOCD, slot)), 0xBADC0DE);

        // Populate balance.
        vm.deal(ALLOCD, 0x1234);
        assertEq(ALLOCD.balance, 0x1234);

        vm.loadAllocs(allocsPath);

        // Info should have changed.
        assertTrue(keccak256(ALLOCD.code) != keccak256(hex"FF"));
        assertEq(uint256(vm.load(ALLOCD, slot)), 0xbeef);
        assertEq(ALLOCD.balance, 0xabcd);
    }

    /// @dev Checks that the `loadAllocs` cheatcode does not override existing account information if there is no data
    ///      within the allocs/genesis file for the account field (i.e., partial overrides)
    function testLoadAllocsPartialOverride() public {
        // Restore the state snapshot prior to the allocs file being loaded.
        vm.revertToState(snapshotId);

        // Populate the alloc'd account's code.
        vm.etch(ALLOCD_B, hex"FF");
        assertEq(ALLOCD_B.code, hex"FF");

        // Populate balance.
        vm.deal(ALLOCD_B, 0x1234);
        assertEq(ALLOCD_B.balance, 0x1234);

        vm.loadAllocs(allocsPath);

        assertEq(ALLOCD_B.code, hex"FF");
        assertEq(ALLOCD_B.balance, 0);
    }
}
