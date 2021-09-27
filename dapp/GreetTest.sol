// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.7.6;

contract Greeter {
    string public greeting;

    function greet(string memory _greeting) public {
        greeting = _greeting;
    }

    function gm() public {
        greeting = "gm";
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

    function testFuzzing(string memory myGreeting) public {
        greeter.greet(myGreeting);
        require(keccak256(abi.encodePacked(greeter.greeting())) == keccak256(abi.encodePacked(myGreeting)), "not equal");
    }

    function testFuzzShrinking(uint256 x, uint256 y) public {
        require(x * y <= 100, "product greater than 100");
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

contract GmTest is GreeterTestSetup {
    function testGm() public {
        greeter.gm();
        require(keccak256(abi.encodePacked(greeter.greeting())) == keccak256(abi.encodePacked("gm")), "not equal");
    }
}
