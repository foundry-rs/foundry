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
        uint256 a = 123;

        bool success = vm.snapshotValue("testSnapshotValue", a);
        assertTrue(success);

        string memory value = vm.readFile("snapshots/testSnapshotValue.json");
        assertEq(value, '"123"');
    }

    function testSnapshotGasSection() public {
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

        string memory value = vm.readFile(
            "snapshots/testSnapshotGasSection.json"
        );
        assertEq(value, '"17439512"');
    }
}
