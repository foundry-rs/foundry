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

    function setUp() public {
        greet = new Greet();
    }

    function testGreeting() public {
        greet.greet("yo");
        require(keccak256(abi.encodePacked(greet.greeting())) == keccak256(abi.encodePacked("yo")), "not equal");
    }

    function testFailGreetingLength() public {
        require(bytes(greet.greeting()).length == 1, "incorrect length");
    }
}
