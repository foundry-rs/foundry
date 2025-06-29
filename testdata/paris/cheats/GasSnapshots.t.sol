// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GasSnapshotTest is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    uint256 public slot0;
    Flare public flare;

    function setUp() public {
        flare = new Flare();
    }

    function testSnapshotGasSectionExternal() public {
        VM.startSnapshotGas("testAssertGasExternal");
        flare.run(1);
        uint256 gasUsed = VM.stopSnapshotGas();

        assertGt(gasUsed, 0);
    }

    function testSnapshotGasSectionInternal() public {
        VM.startSnapshotGas("testAssertGasInternalA");
        slot0 = 1;
        VM.stopSnapshotGas();

        VM.startSnapshotGas("testAssertGasInternalB");
        slot0 = 2;
        VM.stopSnapshotGas();

        VM.startSnapshotGas("testAssertGasInternalC");
        slot0 = 0;
        VM.stopSnapshotGas();

        VM.startSnapshotGas("testAssertGasInternalD");
        slot0 = 1;
        VM.stopSnapshotGas();

        VM.startSnapshotGas("testAssertGasInternalE");
        slot0 = 2;
        VM.stopSnapshotGas();
    }

    // Writes to `GasSnapshotTest` group with custom names.
    function testSnapshotValueDefaultGroupA() public {
        uint256 a = 123;
        uint256 b = 456;
        uint256 c = 789;

        VM.snapshotValue("a", a);
        VM.snapshotValue("b", b);
        VM.snapshotValue("c", c);
    }

    // Writes to same `GasSnapshotTest` group with custom names.
    function testSnapshotValueDefaultGroupB() public {
        uint256 d = 123;
        uint256 e = 456;
        uint256 f = 789;

        VM.snapshotValue("d", d);
        VM.snapshotValue("e", e);
        VM.snapshotValue("f", f);
    }

    // Writes to `CustomGroup` group with custom names.
    // Asserts that the order of the values is alphabetical.
    function testSnapshotValueCustomGroupA() public {
        uint256 o = 123;
        uint256 i = 456;
        uint256 q = 789;

        VM.snapshotValue("CustomGroup", "q", q);
        VM.snapshotValue("CustomGroup", "i", i);
        VM.snapshotValue("CustomGroup", "o", o);
    }

    // Writes to `CustomGroup` group with custom names.
    // Asserts that the order of the values is alphabetical.
    function testSnapshotValueCustomGroupB() public {
        uint256 x = 123;
        uint256 e = 456;
        uint256 z = 789;

        VM.snapshotValue("CustomGroup", "z", z);
        VM.snapshotValue("CustomGroup", "x", x);
        VM.snapshotValue("CustomGroup", "e", e);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGasDefault` name.
    function testSnapshotGasSectionDefaultGroupStop() public {
        VM.startSnapshotGas("testSnapshotGasSection");

        flare.run(256);

        // VM.stopSnapshotGas() will use the last snapshot name.
        uint256 gasUsed = VM.stopSnapshotGas();
        assertGt(gasUsed, 0);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGasCustom` name.
    function testSnapshotGasSectionCustomGroupStop() public {
        VM.startSnapshotGas("CustomGroup", "testSnapshotGasSection");

        flare.run(256);

        // VM.stopSnapshotGas() will use the last snapshot name, even with custom group.
        uint256 gasUsed = VM.stopSnapshotGas();
        assertGt(gasUsed, 0);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGasSection` name.
    function testSnapshotGasSectionName() public {
        VM.startSnapshotGas("testSnapshotGasSectionName");

        flare.run(256);

        uint256 gasUsed = VM.stopSnapshotGas("testSnapshotGasSectionName");
        assertGt(gasUsed, 0);
    }

    // Writes to `CustomGroup` group with `testSnapshotGasSection` name.
    function testSnapshotGasSectionGroupName() public {
        VM.startSnapshotGas("CustomGroup", "testSnapshotGasSectionGroupName");

        flare.run(256);

        uint256 gasUsed = VM.stopSnapshotGas("CustomGroup", "testSnapshotGasSectionGroupName");
        assertGt(gasUsed, 0);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGas` name.
    function testSnapshotGasLastCallName() public {
        flare.run(1);

        uint256 gasUsed = VM.snapshotGasLastCall("testSnapshotGasLastCallName");
        assertGt(gasUsed, 0);
    }

    // Writes to `CustomGroup` group with `testSnapshotGas` name.
    function testSnapshotGasLastCallGroupName() public {
        flare.run(1);

        uint256 gasUsed = VM.snapshotGasLastCall("CustomGroup", "testSnapshotGasLastCallGroupName");
        assertGt(gasUsed, 0);
    }
}

contract GasComparisonTest is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    uint256 public slot0;
    uint256 public slot1;

    uint256 public cachedGas;

    function testGasComparisonEmpty() public {
        // Start a cheatcode snapshot.
        VM.startSnapshotGas("ComparisonGroup", "testGasComparisonEmptyA");
        uint256 a = VM.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        uint256 b = _snapEnd();
        VM.snapshotValue("ComparisonGroup", "testGasComparisonEmptyB", b);

        assertEq(a, b);
    }

    function testGasComparisonInternalCold() public {
        // Start a cheatcode snapshot.
        VM.startSnapshotGas("ComparisonGroup", "testGasComparisonInternalColdA");
        slot0 = 1;
        uint256 a = VM.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        slot1 = 1;
        uint256 b = _snapEnd();
        VM.snapshotValue("ComparisonGroup", "testGasComparisonInternalColdB", b);

        VM.assertApproxEqAbs(a, b, 6);
    }

    function testGasComparisonInternalWarm() public {
        // Warm up the cache.
        slot0 = 1;

        // Start a cheatcode snapshot.
        VM.startSnapshotGas("ComparisonGroup", "testGasComparisonInternalWarmA");
        slot0 = 2;
        uint256 a = VM.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        slot0 = 3;
        uint256 b = _snapEnd();
        VM.snapshotValue("ComparisonGroup", "testGasComparisonInternalWarmB", b);

        VM.assertApproxEqAbs(a, b, 6);
    }

    function testGasComparisonExternal() public {
        // Warm up the cache.
        TargetB target = new TargetB();
        target.update(1);

        // Start a cheatcode snapshot.
        VM.startSnapshotGas("ComparisonGroup", "testGasComparisonExternalA");
        target.update(2);
        uint256 a = VM.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        target.update(3);
        uint256 b = _snapEnd();
        VM.snapshotValue("ComparisonGroup", "testGasComparisonExternalB", b);

        assertEq(a, b);
    }

    function testGasComparisonCreate() public {
        // Start a cheatcode snapshot.
        VM.startSnapshotGas("ComparisonGroup", "testGasComparisonCreateA");
        new TargetC();
        uint256 a = VM.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        new TargetC();
        uint256 b = _snapEnd();
        VM.snapshotValue("ComparisonGroup", "testGasComparisonCreateB", b);

        assertEq(a, b);
    }

    function testGasComparisonNestedCalls() public {
        // Warm up the cache.
        TargetA target = new TargetA();
        target.update(1);

        // Start a cheatcode snapshot.
        VM.startSnapshotGas("ComparisonGroup", "testGasComparisonNestedCallsA");
        target.update(2);
        uint256 a = VM.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        target.update(3);
        uint256 b = _snapEnd();
        VM.snapshotValue("ComparisonGroup", "testGasComparisonNestedCallsB", b);

        assertEq(a, b);
    }

    function testGasComparisonFlare() public {
        // Warm up the cache.
        Flare flare = new Flare();
        flare.run(1);

        // Start a cheatcode snapshot.
        VM.startSnapshotGas("ComparisonGroup", "testGasComparisonFlareA");
        flare.run(256);
        uint256 a = VM.stopSnapshotGas();

        // Start a comparitive Solidity snapshot.
        _snapStart();
        flare.run(256);
        uint256 b = _snapEnd();
        VM.snapshotValue("ComparisonGroup", "testGasComparisonFlareB", b);

        assertEq(a, b);
    }

    // Internal function to start a Solidity snapshot.
    function _snapStart() internal {
        cachedGas = 1;
        cachedGas = gasleft();
    }

    // Internal function to end a Solidity snapshot.
    function _snapEnd() internal returns (uint256 gasUsed) {
        gasUsed = cachedGas - gasleft() - 138;
        cachedGas = 2;
    }
}

contract Flare {
    bytes32[] public data;

    function run(uint256 n_) public {
        for (uint256 i = 0; i < n_; i++) {
            data.push(keccak256(abi.encodePacked(i)));
        }
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

contract TargetC {}
