// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Flare {
    bytes32[] public data;

    function run(uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            data.push(keccak256(abi.encodePacked(i)));
        }
    }
}

contract GasSnapshotTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSnapshotValue() public {
        string memory file = "snapshots/testSnapshotValue.json";
        clear(file);

        uint256 a = 123;

        assertTrue(vm.snapshotValue("testSnapshotValue", a));

        string memory value = vm.readFile(file);
        assertEq(value, '"123"');
    }

    function testSnapshotValueGroup() public {
        string memory file = "snapshots/GasSnapshotTest.json";
        clear(file);

        uint256 a = 123;

        assertTrue(vm.snapshotValue("GasSnapshotTest", "testSnapshotValue", a));

        string memory value = vm.readFile(file);
        assertEq(value, '{\n  "testSnapshotValue": "123"\n}');
    }

    function testSnapshotGroupValue() public {
        string memory file = "snapshots/testSnapshotGroupValue.json";
        clear(file);

        uint256 a = 123;
        uint256 b = 456;
        uint256 c = 789;

        assertTrue(vm.snapshotValue("testSnapshotGroupValue", "a", a));
        assertTrue(vm.snapshotValue("testSnapshotGroupValue", "b", b));

        assertEq(vm.readFile(file), '{\n  "a": "123",\n  "b": "456"\n}');

        assertTrue(vm.snapshotValue("testSnapshotGroupValue", "c", c));

        assertEq(
            vm.readFile(file),
            '{\n  "a": "123",\n  "b": "456",\n  "c": "789"\n}'
        );
    }

    function testSnapshotGasSection() public {
        string memory file = "snapshots/testSnapshotGasSection.json";
        clear(file);

        Flare a = new Flare();

        a.run(64);

        vm.startSnapshotGas("testSnapshotGasSection");

        a.run(256); // 5_821_576 gas
        a.run(512); // 11_617_936 gas

        (bool success, uint256 gasUsed) = vm.stopSnapshotGas(
            "testSnapshotGasSection"
        );
        assertTrue(success);
        assertEq(gasUsed, 17_439_512); // 5_821_576 + 11_617_936 = 17_439_512 gas

        string memory value = vm.readFile(file);
        assertEq(value, '"17439512"');
    }

    // Remove file if it exists so each test can start with a clean slate.
    function clear(string memory name) public {
        if (vm.exists(name)) {
            vm.removeFile(name);
        }
    }
}
