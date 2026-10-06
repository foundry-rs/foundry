// SPDX-License-Identifier: MIT
pragma solidity ^0.8.21;

import {Test} from "forge-std/Test.sol";
import {Migration} from "../src/abstract/Migration.sol";

contract MigrationHarness is Migration {
    constructor(uint24 gracePeriod, address migrator, address initialOwner)
        Migration(gracePeriod, migrator, initialOwner)
    {}
}

contract FarcasterNativeSymbolicTest is Test {
    function check_migrateOnlyMigrator(
        uint24 gracePeriod,
        address migrator,
        address initialOwner,
        address caller,
        uint40 timestamp
    ) public {
        vm.assume(timestamp != 0);

        MigrationHarness migration = new MigrationHarness(gracePeriod, migrator, initialOwner);

        vm.warp(timestamp);
        vm.prank(caller);
        (bool success,) = address(migration).call(abi.encodeCall(Migration.migrate, ()));

        assert(success == (caller == migrator));
        if (success) {
            assert(migration.isMigrated());
            assert(migration.migratedAt() == timestamp);
        }
    }

    function check_setMigratorOnlyOwner(
        uint24 gracePeriod,
        address migrator,
        address initialOwner,
        address caller,
        address nextMigrator
    ) public {
        MigrationHarness migration = new MigrationHarness(gracePeriod, migrator, initialOwner);

        vm.prank(caller);
        (bool success,) =
            address(migration).call(abi.encodeCall(Migration.setMigrator, (nextMigrator)));

        assert(success == (caller == initialOwner));
        if (success) {
            assert(migration.migrator() == nextMigrator);
        } else {
            assert(migration.migrator() == migrator);
        }
    }
}
