// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GasSnapshotTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    Flare public flare;
    uint256 public slot;

    function setUp() public {
        flare = new Flare();
    }

    function testGasExternal() public {
        vm.startSnapshotGas("testAssertGasExternal");

        flare.update(2);

        vm.stopSnapshotGas();
    }

    function testGasInternal() public {
        vm.startSnapshotGas("testAssertGasInternalA");

        slot = 1;

        vm.stopSnapshotGas();

        vm.startSnapshotGas("testAssertGasInternalB");

        slot = 2;

        vm.stopSnapshotGas();

        vm.startSnapshotGas("testAssertGasInternalC");

        slot = 0;

        vm.stopSnapshotGas();

        vm.startSnapshotGas("testAssertGasInternalD");

        slot = 1;

        vm.stopSnapshotGas();

        vm.startSnapshotGas("testAssertGasInternalE");

        slot = 2;

        vm.stopSnapshotGas();
    }

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

    // Writes to `GasSnapshotTest` group with `testSnapshotGasDefault` name.
    function testSnapshotGasSectionDefaultGroupStop() public {
        vm.startSnapshotGas("testSnapshotGasSection");

        flare.run(256);

        // vm.stopSnapshotGas() will use the last snapshot name.
        uint256 gasUsed = vm.stopSnapshotGas();
        assertGt(gasUsed, 0);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGasCustom` name.
    function testSnapshotGasSectionCustomGroupStop() public {
        vm.startSnapshotGas("CustomGroup", "testSnapshotGasSection");

        flare.run(256);

        // vm.stopSnapshotGas() will use the last snapshot name, even with custom group.
        uint256 gasUsed = vm.stopSnapshotGas();
        assertGt(gasUsed, 0);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGasSection` name.
    function testSnapshotGasSectionName() public {
        vm.startSnapshotGas("testSnapshotGasSectionName");

        flare.run(256);

        uint256 gasUsed = vm.stopSnapshotGas("testSnapshotGasSectionName");
        assertGt(gasUsed, 0);
    }

    // Writes to `CustomGroup` group with `testSnapshotGasSection` name.
    function testSnapshotGasSectionGroupName() public {
        vm.startSnapshotGas("CustomGroup", "testSnapshotGasSectionGroupName");

        flare.run(256);

        uint256 gasUsed = vm.stopSnapshotGas("CustomGroup", "testSnapshotGasSectionGroupName");
        assertGt(gasUsed, 0);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGas` name.
    function testSnapshotGasLastCallName() public {
        flare.run(1);

        vm.snapshotGasLastCall("testSnapshotGasName");
    }

    // Writes to `CustomGroup` group with `testSnapshotGas` name.
    function testSnapshotGasLastCallGroupName() public {
        flare.run(1);

        vm.snapshotGasLastCall("CustomGroup", "testSnapshotGasGroupName");
    }
}

contract GasComparisonTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    uint256 public slotA;
    uint256 public slotB;
    uint256 public cachedGas;

    function testGasComparisonEmpty() public {
        // Start a cheatcode snapshot.
        vm.startSnapshotGas("ComparisonGroup", "testGasComparisonEmptyA");
        vm.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        vm.snapshotValue("ComparisonGroup", "testGasComparisonEmptyB", _snapEnd());
    }

    function testGasComparisonInternalCold() public {
        // Start a cheatcode snapshot.
        vm.startSnapshotGas("ComparisonGroup", "testGasComparisonInternalColdA");
        slotA = 1;
        vm.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        slotB = 1;
        vm.snapshotValue("ComparisonGroup", "testGasComparisonInternalColdB", _snapEnd());
    }

    function testGasComparisonInternalWarm() public {
        // Warm up the cache.
        slotA = 1;
        slotB = 1;

        // Start a cheatcode snapshot.
        vm.startSnapshotGas("ComparisonGroup", "testGasComparisonInternalWarmA");
        slotA = 2;
        vm.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        slotB = 2;
        vm.snapshotValue("ComparisonGroup", "testGasComparisonInternalWarmB", _snapEnd());
    }

    function testGasComparisonExternal() public {
        // Warm up the cache.
        TargetB targetA = new TargetB();
        targetA.update(1);
        TargetB targetB = new TargetB();
        targetB.update(1);

        // Start a cheatcode snapshot.
        vm.startSnapshotGas("ComparisonGroup", "testGasComparisonExternalA");
        targetA.update(2);
        vm.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        targetB.update(2);
        vm.snapshotValue("ComparisonGroup", "testGasComparisonExternalB", _snapEnd());
    }

    function testGasComparisonCreateA() public {
        // Start a cheatcode snapshot.
        vm.startSnapshotGas("ComparisonGroup", "testGasComparisonCreateA");
        new TargetEmpty();
        vm.stopSnapshotGas();
    }

    function testGasComparisonCreateB() public {
        // Start a comparitive Solidity snapshot.
        _snapStart();
        new TargetEmpty();
        vm.snapshotValue("ComparisonGroup", "testGasComparisonCreateB", _snapEnd());
    }

    // Internal function to start a Solidity snapshot.
    function _snapStart() internal {
        cachedGas = 1;
        cachedGas = gasleft();
    }

    // Internal function to end a Solidity snapshot.
    function _snapEnd() internal returns (uint256 gasUsed) {
        gasUsed = cachedGas - gasleft() - 174;
        cachedGas = 2;
    }
}

contract Flare {
    TargetA public target;
    bytes32[] public data;

    constructor() {
        target = new TargetA();
    }

    function run(uint256 n_) public {
        for (uint256 i = 0; i < n_; i++) {
            data.push(keccak256(abi.encodePacked(i)));
        }
    }

    function update(uint256 x_) public {
        target.update(x_);
    }
}

contract TargetA {
    TargetB public target;

    constructor() {
        target = new TargetB();
    }

    function update(uint256 x_) public {
        target.update(x_);
    }
}

contract TargetB {
    uint256 public x;

    function update(uint256 x_) public {
        x = x_;
    }
}

contract TargetEmpty {}
