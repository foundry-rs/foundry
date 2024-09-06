// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Counter {
    uint256 public a;
    address public b;

    function setA(uint256 _a) public {
        a = _a;
    }

    function setB(address _b) public {
        b = _b;
    }
}

contract CounterTest is DSTest {
    Counter public counter;
    Counter public counter1;
    Vm vm = Vm(HEVM_ADDRESS);

    function setUp() public {
        counter = new Counter();
        counter.setA(1000);
        counter.setB(address(27));
        counter1 = new Counter();
        counter1.setA(11);
        counter1.setB(address(50));
    }

    function test_copy_storage() public {
        assertEq(counter.a(), 1000);
        assertEq(counter.b(), address(27));
        assertEq(counter1.a(), 11);
        assertEq(counter1.b(), address(50));
        vm.copyStorage(address(counter), address(counter1));
        assertEq(counter.a(), 1000);
        assertEq(counter.b(), address(27));
        assertEq(counter1.a(), 1000);
        assertEq(counter1.b(), address(27));
    }
}

contract CopyStorageContract {
    uint256 public x;
}

contract CopyStorageTest is DSTest {
    CopyStorageContract csc_1;
    CopyStorageContract csc_2;
    Vm vm = Vm(HEVM_ADDRESS);

    function _storeUInt256(address contractAddress, uint256 slot, uint256 value) internal {
        vm.store(contractAddress, bytes32(slot), bytes32(value));
    }

    function setUp() public {
        csc_1 = new CopyStorageContract();
        csc_2 = new CopyStorageContract();
    }

    function test_copy_storage() public {
        // Make the storage of first contract symbolic
        vm.setArbitraryStorage(address(csc_1));
        // and explicitly put a constrained symbolic value into the slot for `x`
        uint256 x_1 = vm.randomUint();
        _storeUInt256(address(csc_1), 0, x_1);
        // `x` of second contract is uninitialized
        assert(csc_2.x() == 0);
        // Copy storage from first to second contract
        vm.copyStorage(address(csc_1), address(csc_2));
        // `x` of second contract is now the `x` of the first
        assert(csc_2.x() == x_1);
    }
}
