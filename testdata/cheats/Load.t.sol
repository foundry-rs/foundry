// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

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
        vm.expectRevert(bytes("cannot call `load` on precompile 0x0000000000000000000000000000000000000001"));
        uint256 val = this.load(address(1), bytes32(0));
    }

    function load(address target, bytes32 slot) public returns (uint256) {
        return uint256(vm.load(target, slot));
    }

    function testLoadOtherStorage() public {
        uint256 val = uint256(vm.load(address(store), bytes32(0)));
        assertEq(val, 10, "load failed");
    }
}
