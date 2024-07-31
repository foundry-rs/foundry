// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0 <0.9.0;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract SimpleStorage {
    uint256 public value;

    function set(uint256 _value) external {
        value = _value;
    }
}

contract Issue5935Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testFork() public {
        uint256 forkId1 = vm.createFork("mainnet", 18234083);
        uint256 forkId2 = vm.createFork("mainnet", 18234083);
        vm.selectFork(forkId1);
        SimpleStorage myContract = new SimpleStorage();
        myContract.set(42);
        vm.selectFork(forkId2);
        SimpleStorage myContract2 = new SimpleStorage();
        assertEq(myContract2.value(), 0);

        vm.selectFork(forkId1);
        assertEq(myContract.value(), 42);
    }
}
