// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.7.6;

contract Greeter {
    string public greeting;

    function greet(string memory _greeting) public {
        greeting = _greeting;
    }

    function time() public view returns (uint256) {
        return block.timestamp;
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

interface HEVM {
    function warp(uint256 time) external;
}

address constant HEVM_ADDRESS =
    address(bytes20(uint160(uint256(keccak256('hevm cheat code')))));

contract GreeterTest is GreeterTestSetup {
    HEVM constant hevm = HEVM(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    function greet(string memory _greeting) public {
        greeter.greet(_greeting);
    }

    function testHevmTime() public {
        uint256 val = 100;
        hevm.warp(100);
        uint256 timestamp = greeter.time();
        require(timestamp == val);
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
