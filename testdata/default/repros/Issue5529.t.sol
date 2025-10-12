// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "utils/Test.sol";

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}

/// forge-config: default.always_use_create_2_factory = true
contract Issue5529Test is Test {
    Counter public counter;
    address public constant default_create2_factory = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    function testCreate2FactoryUsedInTests() public {
        run();
    }

    function testCreate2FactoryUsedWhenPranking() public {
        vm.startPrank(address(1234));
        run();
    }

    function run() private {
        address a = vm.computeCreate2Address(0, keccak256(type(Counter).creationCode), address(default_create2_factory));
        address b = address(new Counter{salt: 0}());
        require(a == b, "create2 address mismatch");
    }
}
