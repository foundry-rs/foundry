pragma solidity 0.8.10;

interface IContract { function foo() external; }

// your 2 implementations
contract A is IContract { function foo() public {  } }
contract B is IContract { function foo() public {  } }

// the shared test suite
abstract contract Tests {
          IContract myContract;
          // this function is part of any contract that inherits `Tests`
          function testFoo() public { myContract.foo(); }
}

contract ATests is Tests {
         function setUp() public {
                  myContract = IContract(address(new A()));
         }
}

contract BTests is Tests {
         function setUp() public {
                  myContract = IContract(address(new B()));
         }
}
