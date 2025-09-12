// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}

contract Issue5529Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    Counter public counter;
    address public constant default_create2_factory = 0xba5Ed099633D3B313e4D5F7bdc1305d3c28ba5Ed;

    function testCreate2FactoryUsedInTests() public {
        address a = vm.computeCreate2Address(0, keccak256(type(Counter).creationCode), address(default_create2_factory));
        address b = address(new Counter{salt: 0}());
        require(a == b, "create2 address mismatch");
    }

    function testCreate2FactoryUsedWhenPranking() public {
        vm.startPrank(address(1234));
        address a = vm.computeCreate2Address(0, keccak256(type(Counter).creationCode), address(default_create2_factory));
        address b = address(new Counter{salt: 0}());
        require(a == b, "create2 address mismatch");
    }
}
