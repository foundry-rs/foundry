// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GasSnapshotTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSnapshotValue() public {
        uint256 a = 123;
        uint256 b = 456;
        uint256 c = 789;

        vm.snapshotValue("snapshotValueA", a);
        vm.snapshotValue("snapshotValueB", b);
        vm.snapshotValue("snapshotValueC", c);

        // Overwrite a
        uint256 a2 = 321;

        vm.snapshotValue("snapshotValueA", a2);
    }

    function testSnapshotGasSection() public {
        Flare f = new Flare();

        f.run(1);

        vm.startSnapshotGas("testSnapshotGasSection");

        f.run(256); // 5_821_576 gas
        f.run(512); // 11_617_936 gas

        uint256 gasUsed = vm.stopSnapshotGas("testSnapshotGasSection");
        assertEq(gasUsed, 17_439_512); // 5_821_576 + 11_617_936 = 17_439_512 gas
    }

    function testSnapshotOrdering() public {
        uint256 a = 123;
        uint256 b = 456;
        uint256 c = 789;

        vm.snapshotValue("SnapshotOrdering", "c", c);
        vm.snapshotValue("SnapshotOrdering", "a", a);
        vm.snapshotValue("SnapshotOrdering", "b", b);
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
