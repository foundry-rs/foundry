// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GasSnapshotTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    // Writes to `GasSnapshotTest` group with custom names.
    function testSnapshotValueDefaultGroup1() public {
        uint256 a = 123;
        uint256 b = 456;
        uint256 c = 789;

        vm.snapshotValue("a", a);
        vm.snapshotValue("b", b);
        vm.snapshotValue("c", c);
    }

    // Writes to same `GasSnapshotTest` group with custom names.
    function testSnapshotValueDefaultGroup2() public {
        uint256 d = 123;
        uint256 e = 456;
        uint256 f = 789;

        vm.snapshotValue("d", d);
        vm.snapshotValue("e", e);
        vm.snapshotValue("f", f);
    }

    // Writes to `CustomGroup` group with custom names.
    // Asserts that the order of the values is alphabetical.
    function testSnapshotValueCustomGroup1() public {
        uint256 o = 123;
        uint256 i = 456;
        uint256 q = 789;

        vm.snapshotValue("CustomGroup", "q", q);
        vm.snapshotValue("CustomGroup", "i", i);
        vm.snapshotValue("CustomGroup", "o", o);
    }

    // Writes to `CustomGroup` group with custom names.
    // Asserts that the order of the values is alphabetical.
    function testSnapshotValueCustomGroup2() public {
        uint256 x = 123;
        uint256 e = 456;
        uint256 z = 789;

        vm.snapshotValue("CustomGroup", "z", z);
        vm.snapshotValue("CustomGroup", "x", x);
        vm.snapshotValue("CustomGroup", "e", e);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGasSection` name.
    function testSnapshotGasSectionName() public {
        Flare f = new Flare();

        vm.startSnapshotGas("testSnapshotGasSectionName");

        f.run(256);

        uint256 gasUsed = vm.stopSnapshotGas("testSnapshotGasSectionName");
        assertGt(gasUsed, 0);
    }

    // Writes to `CustomGroup` group with `testSnapshotGasSection` name.
    function testSnapshotGasSectionGroupName() public {
        Flare f = new Flare();

        vm.startSnapshotGas("CustomGroup", "testSnapshotGasSectionGroupName");

        f.run(256);

        uint256 gasUsed = vm.stopSnapshotGas("CustomGroup", "testSnapshotGasSectionGroupName");
        assertGt(gasUsed, 0);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGas` name.
    function testSnapshotGasName() public {
        Flare f = new Flare();

        f.run(1);

        vm.snapshotGas("testSnapshotGasName");
    }

    // Writes to `CustomGroup` group with `testSnapshotGas` name.
    function testSnapshotGasGroupName() public {
        Flare f = new Flare();

        f.run(1);

        vm.snapshotGas("CustomGroup", "testSnapshotGasGroupName");
    }
}

contract Flare {
    bytes32[] public data;

    function run(uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            data.push(keccak256(abi.encodePacked(i)));
        }
    }
}
