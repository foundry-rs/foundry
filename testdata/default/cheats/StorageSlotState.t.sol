// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract StorageSlotStateTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_gas_two_reads() public {
        Read read = new Read();
        read.number();
        uint256 initial = gasleft();
        read.number();
        assert(initial - gasleft() >= 614);
    }

    function test_gas_mark_warm() public {
        Read read = new Read();
        vm.warmSlot(address(read), bytes32(0));
        uint256 initial = gasleft();
        read.number();
        assert(initial - gasleft() >= 614);
    }

    function test_gas_mark_cold() public {
        Read read = new Read();
        read.number();
        vm.coolSlot(address(read), bytes32(0));
        uint256 initial = gasleft();
        read.number();
        assert(initial - gasleft() >= 2614);
    }
}

contract Read {
    uint256 public number = 10;
}
