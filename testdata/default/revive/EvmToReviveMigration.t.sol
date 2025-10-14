// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract SimpleStorage {
    uint256 private value;

    function set(uint256 _value) public {
        value = _value;
    }

    function get() public view returns (uint256) {
        return value;
    }
}

contract EvmReviveMigrationTest is DSTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address alice = address(0x1111);

    function setUp() public {
        vm.deal(alice, 1 ether);
        // Mark accounts as persistent so they migrate when switching between EVM and PVM
        vm.makePersistent(alice);
    }

    function testBalanceMigration() public {
        // Tests run in Revive by default when using runner_revive
        vm.deal(alice, 3 ether);
        uint256 reviveBalance = alice.balance;
        assertEq(reviveBalance, 3 ether, "Revive balance should be 3 ether");

        vm.pvm(false);

        assertEq(alice.balance, reviveBalance, "Balance should migrate from Revive to EVM");

        vm.deal(alice, 2 ether);
        uint256 evmBalance = alice.balance;
        assertEq(evmBalance, 2 ether, "Revive balance should be 2 ether");

        vm.pvm(true);

        assertEq(alice.balance, evmBalance, "Balance should migrate from EVM to Revive");
    }

    function testNonceMigration() public {
        vm.setNonce(alice, 5);
        uint256 reviveNonce = vm.getNonce(alice);
        assertEq(reviveNonce, 5, "Nonce in Revive should be 5");

        vm.pvm(false);

        assertEq(vm.getNonce(alice), reviveNonce, "Nonce should migrate from Revive to EVM");

        vm.setNonce(alice, 10);
        uint256 evmNonce = vm.getNonce(alice);
        assertEq(evmNonce, 10, "Nonce in Revive should be 10");

        vm.pvm(true);
        assertEq(vm.getNonce(alice), evmNonce, "Nonce should migrate from EVM to Revive");
    }

    function testPrecisionPreservation() public {
        // Set precise balance in Revive (with wei precision)
        vm.deal(alice, 1123456789123456789);
        uint256 reviveBalance = alice.balance;
        assertEq(reviveBalance, 1123456789123456789, "Balance should be set correctly in Revive");

        vm.pvm(false);

        assertEq(alice.balance, 1123456789123456789, "Balance precision should be preserved in migration to EVM");

        vm.deal(alice, 1123456789123456790);
        uint256 evmBalance = alice.balance;
        assertEq(evmBalance, 1123456789123456790, "Balance should be set correctly in EVM");

        vm.pvm(true);
        assertEq(alice.balance, evmBalance, "Balance precision should be preserved in migration back to Revive");
    }

    function testBytecodeMigration() public {
        SimpleStorage storageContract = new SimpleStorage();

        // Mark the contract as persistent so it migrates
        vm.makePersistent(address(storageContract));

        storageContract.set(42);
        assertEq(storageContract.get(), 42);

        vm.pvm(false);

        assertEq(storageContract.get(), 42);

        storageContract.set(100);

        assertEq(storageContract.get(), 100);
    }

    function testTimestampMigration() public {
        uint256 initialTimestamp = 1_000_000;
        vm.warp(initialTimestamp);

        uint256 reviveTimestamp = block.timestamp;
        assertEq(reviveTimestamp, initialTimestamp, "Timestamp in Revive should match initial value");

        vm.pvm(false);

        uint256 evmTimestamp = block.timestamp;
        assertEq(evmTimestamp, reviveTimestamp, "Timestamp should migrate from Revive to EVM");

        uint256 newEvmTimestamp = 2_000_000_000;
        vm.warp(newEvmTimestamp);
        assertEq(block.timestamp, newEvmTimestamp, "Timestamp in EVM should update correctly");

        vm.pvm(true);

        uint256 finalReviveTimestamp = block.timestamp;
        assertEq(finalReviveTimestamp, newEvmTimestamp, "Timestamp should migrate from EVM to Revive");
    }
}
