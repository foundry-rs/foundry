// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Counter {
    uint256 public a;
    address public b;
    int256[] public c;

    function setA(uint256 _a) public {
        a = _a;
    }

    function setB(address _b) public {
        b = _b;
    }
}

contract CounterWithSeedTest is DSTest {
    Counter public counter;
    Counter public counter1;
    Vm vm = Vm(HEVM_ADDRESS);

    function test_copy_storage() public {
        counter = new Counter();
        counter.setA(1000);
        counter.setB(address(27));
        counter1 = new Counter();
        counter1.setA(11);
        counter1.setB(address(50));

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

    function test_copy_storage_from_arbitrary() public {
        counter = new Counter();
        counter1 = new Counter();
        vm.setArbitraryStorage(address(counter));
        vm.copyStorage(address(counter), address(counter1));

        // Make sure untouched storage has same values.
        assertEq(counter.a(), counter1.a());
        assertEq(counter.b(), counter1.b());
        assertEq(counter.c(33), counter1.c(33));

        // Change storage in source storage contract and make sure copy is not changed.
        counter.setA(1000);
        counter1.setB(address(50));
        assertEq(counter.a(), 1000);
        assertEq(counter1.a(), 67350900536747027229585709178274816969402970928486983076982664581925078789474);
        assertEq(counter.b(), 0x5A61ACa23C478d83A72425c386Eb5dB083FBd0e4);
        assertEq(counter1.b(), address(50));
    }
}

contract CopyStorageContract {
    uint256 public x;
}

contract CopyStorageTest is DSTest {
    CopyStorageContract csc_1;
    CopyStorageContract csc_2;
    CopyStorageContract csc_3;
    Vm vm = Vm(HEVM_ADDRESS);

    function _storeUInt256(address contractAddress, uint256 slot, uint256 value) internal {
        vm.store(contractAddress, bytes32(slot), bytes32(value));
    }

    function setUp() public {
        csc_1 = new CopyStorageContract();
        csc_2 = new CopyStorageContract();
        csc_3 = new CopyStorageContract();
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

    function test_copy_storage_same_values_on_load() public {
        // Make the storage of first contract symbolic
        vm.setArbitraryStorage(address(csc_1));
        vm.copyStorage(address(csc_1), address(csc_2));
        uint256 slot1 = vm.randomUint(0, 100);
        uint256 slot2 = vm.randomUint(0, 100);
        bytes32 value1 = vm.load(address(csc_1), bytes32(slot1));
        bytes32 value2 = vm.load(address(csc_1), bytes32(slot2));

        bytes32 value3 = vm.load(address(csc_2), bytes32(slot1));
        bytes32 value4 = vm.load(address(csc_2), bytes32(slot2));

        // Check storage values are the same for both source and target contracts.
        assertEq(value1, value3);
        assertEq(value2, value4);
    }

    function test_copy_storage_consistent_values() public {
        // Make the storage of first contract symbolic.
        vm.setArbitraryStorage(address(csc_1));
        // Copy arbitrary storage to 2 contracts.
        vm.copyStorage(address(csc_1), address(csc_2));
        vm.copyStorage(address(csc_1), address(csc_3));
        uint256 slot1 = vm.randomUint(0, 100);
        uint256 slot2 = vm.randomUint(0, 100);

        // Load slot 1 from 1st copied contract and slot2 from symbolic contract.
        bytes32 value3 = vm.load(address(csc_2), bytes32(slot1));
        bytes32 value2 = vm.load(address(csc_1), bytes32(slot2));

        bytes32 value1 = vm.load(address(csc_1), bytes32(slot1));
        bytes32 value4 = vm.load(address(csc_2), bytes32(slot2));

        // Make sure same values for both copied and symbolic contract.
        assertEq(value3, value1);
        assertEq(value2, value4);

        uint256 x_1 = vm.randomUint();
        // Change slot1 of 1st copied contract.
        _storeUInt256(address(csc_2), slot1, x_1);
        value3 = vm.load(address(csc_2), bytes32(slot1));
        bytes32 value5 = vm.load(address(csc_3), bytes32(slot1));
        // Make sure value for 1st contract copied is different than symbolic contract value.
        assert(value3 != value1);
        // Make sure same values for 2nd contract copied and symbolic contract.
        assertEq(value5, value1);

        uint256 x_2 = vm.randomUint();
        // Change slot2 of symbolic contract.
        _storeUInt256(address(csc_1), slot2, x_2);
        value2 = vm.load(address(csc_1), bytes32(slot2));
        bytes32 value6 = vm.load(address(csc_3), bytes32(slot2));
        // Make sure value for symbolic contract value is different than 1st contract copied.
        assert(value2 != value4);
        // Make sure value for symbolic contract value is different than 2nd contract copied.
        assert(value2 != value6);
        assertEq(value4, value6);
    }
}
