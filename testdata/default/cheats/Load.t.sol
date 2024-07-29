// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Storage {
    uint256 slot0 = 10;
}

contract LoadTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 slot0 = 20;
    Storage store;

    function setUp() public {
        store = new Storage();
    }

    function testLoadOwnStorage() public {
        uint256 slot;
        assembly {
            slot := slot0.slot
        }
        uint256 val = uint256(vm.load(address(this), bytes32(slot)));
        assertEq(val, 20, "load failed");
    }

    function testLoadNotAvailableOnPrecompiles() public {
        vm._expectCheatcodeRevert("cannot use precompile 0x0000000000000000000000000000000000000001 as an argument");
        vm.load(address(1), bytes32(0));
    }

    function testLoadOtherStorage() public {
        uint256 val = uint256(vm.load(address(store), bytes32(0)));
        assertEq(val, 10, "load failed");
    }
}
