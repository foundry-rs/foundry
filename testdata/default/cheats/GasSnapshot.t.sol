// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";
import "../logs/console.sol";

contract GasSnapshotTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    /// @notice Gas overhead for the Solidity snapshotting function itself.
    uint256 private constant GAS_CALIBRATION = 100;

    /// @notice Transient variable for the start gas.
    uint256 private cachedGas;

    /// @notice Transient variable for the snapshot name.
    string private cachedName;

    /// @notice Arbitrary slot to write to.
    uint256 private slot;

    function testGasExternal() public {
        Flare f = new Flare();

        vm.startSnapshotGas("testAssertGasExternal");

        f.update(2);

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

    function testGasComparison() public {
        TargetB target = new TargetB();

        // Warm up the cache.
        target.update(1);

        // Start a cheatcode snapshot.
        vm.startSnapshotGas("ComparisonGroup", "testGasComparisonA");

        target.update(2);

        uint256 gasA = vm.stopSnapshotGas();
        console.log("gas A", gasA);

        // Start a comparitive Solidity snapshot.
        _snapStart("testGasComparisonB");

        target.update(3);

        uint256 gasB = _snapEnd();
        console.log("gas B", gasB);

        vm.snapshotValue("ComparisonGroup", "testGasComparisonB", gasB);
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
        Flare f = new Flare();

        vm.startSnapshotGas("testSnapshotGasSection");

        f.run(256);

        // vm.stopSnapshotGas() will use the last snapshot name.
        uint256 gasUsed = vm.stopSnapshotGas();
        assertGt(gasUsed, 0);
    }

    // Writes to `GasSnapshotTest` group with `testSnapshotGasCustom` name.
    function testSnapshotGasSectionCustomGroupStop() public {
        Flare f = new Flare();

        vm.startSnapshotGas("CustomGroup", "testSnapshotGasSection");

        f.run(256);

        // vm.stopSnapshotGas() will use the last snapshot name, even with custom group.
        uint256 gasUsed = vm.stopSnapshotGas();
        assertGt(gasUsed, 0);
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
    function testSnapshotGasLastCallName() public {
        Flare f = new Flare();

        f.run(1);

        vm.snapshotGasLastCall("testSnapshotGasName");
    }

    // Writes to `CustomGroup` group with `testSnapshotGas` name.
    function testSnapshotGasLastCallGroupName() public {
        Flare f = new Flare();

        f.run(1);

        vm.snapshotGasLastCall("CustomGroup", "testSnapshotGasGroupName");
    }

    function _snapStart(string memory name) internal {
        // Warm up cachedGas so the only sstore after calling `gasleft` is exactly 100 gas
        cachedGas = 1;
        cachedName = name;
        cachedGas = gasleft();
    }

    function _snapEnd() internal returns (uint256 gasUsed) {
        uint256 newGasLeft = gasleft();
        gasUsed = cachedGas - newGasLeft - GAS_CALIBRATION;
        cachedGas = 0;
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
