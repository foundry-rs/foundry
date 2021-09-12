// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.6;

contract Greeter {
    string public greeting;

    function greet(string memory _greeting) public {
        greeting = _greeting;
    }
}

contract GreeterTestSetup {
    Greeter greeter;

    function greeting() public view returns (string memory) {
        return greeter.greeting();
    }

    function setUp() public {
        greeter = new Greeter();
    }
}

contract GreeterTest is GreeterTestSetup {
    function greet(string memory greeting) public {
        greeter.greet(greeting);
    }

    // check the positive case
    function testGreeting() public {
        greeter.greet("yo");
        require(keccak256(abi.encodePacked(greeter.greeting())) == keccak256(abi.encodePacked("yo")), "not equal");
    }

    // check the unhappy case
    function testFailGreeting() public {
        greeter.greet("yo");
        require(keccak256(abi.encodePacked(greeter.greeting())) == keccak256(abi.encodePacked("hi")), "not equal to `hi`");
    }

    function testIsolation() public {
        require(bytes(greeter.greeting()).length == 0);
    }
}
