// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GasSnapshotTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSnapshotValue() public {
        string memory file = "snapshots/testSnapshotValue.json";
        clear(file);

        uint256 a = 123;

        assertTrue(vm.snapshotValue("testSnapshotValue", "a", a));

        // Expect:
        // {
        //   "a": "123"
        // }
        string memory value = vm.readFile(file);
        assertEq(value, '{\n  "a": "123"\n}');
    }

    function testSnapshotValueGroup() public {
        string memory file = "snapshots/testSnapshotGroupValue.json";
        clear(file);

        uint256 a = 123;
        uint256 b = 456;
        uint256 c = 789;

        assertTrue(vm.snapshotValue("testSnapshotGroupValue", "a", a));
        assertTrue(vm.snapshotValue("testSnapshotGroupValue", "b", b));

        // Expect:
        // {
        //   "a": "123",
        //   "b": "456"
        // }
        assertEq(vm.readFile(file), '{\n  "a": "123",\n  "b": "456"\n}');

        assertTrue(vm.snapshotValue("testSnapshotGroupValue", "c", c));

        // Expect:
        // {
        //   "a": "123",
        //   "b": "456",
        //   "c": "789"
        // }
        assertEq(vm.readFile(file), '{\n  "a": "123",\n  "b": "456",\n  "c": "789"\n}');

        // Overwrite a
        uint256 a2 = 321;

        assertTrue(vm.snapshotValue("testSnapshotGroupValue", "a", a2));

        // Expect:
        // {
        //   "a": "321",
        //   "b": "456",
        //   "c": "789"
        // }
        assertEq(vm.readFile(file), '{\n  "a": "321",\n  "b": "456",\n  "c": "789"\n}');
    }

    function testSnapshotGasSection() public {
        string memory file = "snapshots/testSnapshotGasSection.json";
        clear(file);

        Flare f = new Flare();

        f.run(1);

        vm.startSnapshotGas("testSnapshotGasSection", "a");

        f.run(256); // 5_821_576 gas
        f.run(512); // 11_617_936 gas

        (bool success, uint256 gasUsed) = vm.stopSnapshotGas("testSnapshotGasSection", "a");
        assertTrue(success);
        assertEq(gasUsed, 17_439_512); // 5_821_576 + 11_617_936 = 17_439_512 gas

        // Expect:
        // {
        //   "a": "17439512"
        // }
        assertEq(vm.readFile(file), '{\n  "a": "17439512"\n}');
    }

    function testSnapshotOrdering() public {
        string memory file = "snapshots/SnapshotOrdering.json";
        clear(file);

        uint256 a = 123;
        uint256 b = 456;
        uint256 c = 789;

        vm.snapshotValue("SnapshotOrdering", "c", c);

        // Expect:
        // {
        //   "c": "789"
        // }
        assertEq(vm.readFile(file), '{\n  "c": "789"\n}');

        vm.snapshotValue("SnapshotOrdering", "a", a);

        // Expect:
        // {
        //   "a": "123",
        //   "c": "789"
        // }
        assertEq(vm.readFile(file), '{\n  "a": "123",\n  "c": "789"\n}');

        vm.snapshotValue("SnapshotOrdering", "b", b);

        // Expect:
        // {
        //   "a": "123",
        //   "b": "456",
        //   "c": "789"
        // }
        assertEq(vm.readFile(file), '{\n  "a": "123",\n  "b": "456",\n  "c": "789"\n}');
    }

    function testSnapshotCombination() public {
        string memory file = "snapshots/SnapshotCombination.json";
        clear(file);

        uint256 a = 123;
        uint256 b = 456;
        uint256 c = 789;

        vm.snapshotValue("SnapshotCombination", "c", c);
        vm.snapshotValue("SnapshotCombination", "a", a);

        Flare f = new Flare();

        f.run(1);

        vm.startSnapshotGas("SnapshotCombination", "z");

        f.run(256); // 5_821_576 gas

        (bool success, uint256 gasUsed) = vm.stopSnapshotGas("SnapshotCombination", "z");
        assertTrue(success);
        assertEq(gasUsed, 5_821_576);

        vm.snapshotValue("SnapshotCombination", "b", b);

        // Expect:
        // {
        //   "a": "123",
        //   "b": "456",
        //   "c": "789",
        //   "z": "5821576"
        // }
        assertEq(vm.readFile(file), '{\n  "a": "123",\n  "b": "456",\n  "c": "789",\n  "z": "5821576"\n}');
    }

    // Remove file if it exists so each test can start with a clean slate.
    function clear(string memory name) public {
        if (vm.exists(name)) {
            vm.removeFile(name);
        }
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
