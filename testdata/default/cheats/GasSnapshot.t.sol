// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GasSnapshotTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    // function testFuzzSnapshotValue1(string memory a, uint256 b) public {
    //     vm.snapshotValue(a, b);
    // }

    function testSnapshotValue1() public {
        uint256 a = 123;
        uint256 b = 456;
        uint256 c = 789;

        vm.snapshotValue("a", a);
        vm.snapshotValue("b", b);
        vm.snapshotValue("c", c);
    }

    function testSnapshotValue2() public {
        uint256 d = 123;
        uint256 e = 456;
        uint256 f = 789;

        vm.snapshotValue("d", d);
        vm.snapshotValue("e", e);
        vm.snapshotValue("f", f);
    }

    // function testSnapshotValue2() public {
    //     uint256 a = 123;
    //     uint256 b = 456;
    //     uint256 c = 789;

    //     vm.snapshotValue("snapshotValue2A", a);
    //     vm.snapshotValue("snapshotValue2B", b);
    //     vm.snapshotValue("snapshotValue2C", c);

    //     // Overwrite a
    //     uint256 a2 = 321;

    //     vm.snapshotValue("snapshotValue2A", a2);
    // }

    // function testSnapshotValue3() public {
    //     vm.snapshotValue("snapshotValue3A", 1);
    //     vm.snapshotValue("snapshotValue3B", 2);
    //     vm.snapshotValue("snapshotValue3C", 3);
    // }

    // function testSnapshotGasSection() public {
    //     Flare f = new Flare();

    //     f.run(1);

    //     vm.startSnapshotGas("testSnapshotGasSection");

    //     f.run(256); // 5_821_576 gas
    //     f.run(512); // 11_617_936 gas

    //     uint256 gasUsed = vm.stopSnapshotGas("testSnapshotGasSection");
    //     assertEq(gasUsed, 17_439_512); // 5_821_576 + 11_617_936 = 17_439_512 gas
    // }

    // function testSnapshotOrdering() public {
    //     uint256 a = 123;
    //     uint256 b = 456;
    //     uint256 c = 789;

    //     vm.snapshotValue("SnapshotOrdering", "c", c);
    //     vm.snapshotValue("SnapshotOrdering", "a", a);
    //     vm.snapshotValue("SnapshotOrdering", "b", b);
    // }
}

contract Flare {
    bytes32[] public data;

    function run(uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            data.push(keccak256(abi.encodePacked(i)));
        }
    }
}
