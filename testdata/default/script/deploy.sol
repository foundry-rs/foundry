// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Greeter {
    string name;
    uint256 age;

    event Greet(string greet);

    function greeting(string memory _name) public returns (string memory) {
        name = _name;
        string memory greet = string(abi.encodePacked("Hello ", _name));
        emit Greet(greet);
        return greet;
    }

    function setAge(uint256 _age) external {
        age = _age;
    }
}

contract Deploy is Test {
    Greeter greeter;
    string greeting;

    function run() external {
        vm.startBroadcast();
        greeter = new Greeter();
        greeting = greeter.greeting("john");
        greeter.setAge(123);
        vm.stopBroadcast();
    }
}
