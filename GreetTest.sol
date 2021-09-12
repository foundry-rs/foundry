// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.6;

contract Greet {
    string public greeting;

    function greet(string memory _greeting) public {
        greeting = _greeting;
    }
}

contract GreetTest {
    Greet greet;

    function greeting() public view returns (string memory) {
        return greet.greeting();
    }

    function setUp() public {
        greet = new Greet();
    }

    function testIsolation() public {
        require(bytes(greet.greeting()).length == 0);
    }

    // check the positive case
    function testGreeting() public {
        greet.greet("yo");
        require(keccak256(abi.encodePacked(greet.greeting())) == keccak256(abi.encodePacked("yo")), "not equal");
    }

    // check the unhappy case
    function testFailGreeting() public {
        greet.greet("yo");
        require(keccak256(abi.encodePacked(greet.greeting())) == keccak256(abi.encodePacked("hi")), "not equal to `hi`");
    }
}
