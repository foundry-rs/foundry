// SPDX-License-Identifier: UNLICENSED
pragma abicoder v2;
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

    function testShrinking(uint256 x, uint256 y) public {
        require(x * y <= 100, "product greater than 100");
    }


    function testFuzzString(string memory myGreeting) public {
        greeter.greet(myGreeting);
        require(keccak256(abi.encodePacked(greeter.greeting())) == keccak256(abi.encodePacked(myGreeting)), "not equal");
    }

    function testFuzzFixedArray(uint256[2] memory x) public {
        if (x[0] == 0) return;
        require(x[1] / x[1] == 0);
    }

    function testFuzzVariableArray(uint256[] memory x) public {
        if (x.length < 2) return;
        if (x[0] == 0) return;
        require(x[1] / x[1] == 0);
    }

    function testFuzzBytes1(bytes1 x) public {
        require(x == 0);
    }

    function testFuzzBytes14(bytes14 x) public {
        require(x == 0);
    }

    function testFuzzBytes32(bytes32 x) public {
        require(x == 0);
    }

    function testFuzzI256(int256 x) public {
        require(x >= 0);
    }

    struct Foo {
        Bar bar;
    }

    struct Bar {
        uint256 baz;
    }

    function testFuzzAbiCoderV2(Foo memory foo) public {
        require(foo.bar.baz < 5);
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
