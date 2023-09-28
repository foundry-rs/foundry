// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0 <0.9.0;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

contract SimpleStorage {
    uint256 public value;

    function set(uint256 _value) external {
        value = _value;
    }
}

/// @dev start anvil --port 35353
contract Issue5935Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testFork() public {
        uint256 forkId1 =
            vm.createFork("https://eth-mainnet.alchemyapi.io/v2/QC55XC151AgkS3FNtWvz9VZGeu9Xd9lb", 18234083);
        uint256 forkId2 =
            vm.createFork("https://eth-mainnet.alchemyapi.io/v2/QC55XC151AgkS3FNtWvz9VZGeu9Xd9lb", 18234083);
        vm.selectFork(forkId1);
        SimpleStorage myContract = new SimpleStorage();
        emit log_address(address(myContract));
        vm.selectFork(forkId2);
        SimpleStorage myContract2 = new SimpleStorage();
        emit log_address(address(myContract2));
    }
}
