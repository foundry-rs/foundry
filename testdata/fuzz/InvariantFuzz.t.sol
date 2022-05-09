// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract UnbreakableMockToken {

    uint public totalSupply = 0;

    function mint(uint _a) public {
        totalSupply *= _a;
    } 
}

contract BreakableMockToken {

    uint public totalSupply = 0;

    function mint(uint _a) public {
        totalSupply += 5;
    } 
}

contract InvariantFuzzTest is DSTest {

    UnbreakableMockToken unbreakable;
    BreakableMockToken breakable;

  function setUp() public {
      unbreakable = new UnbreakableMockToken();
      breakable = new BreakableMockToken();
  }

  function invariantTestPass() public {
    emit log("invariantTestPassLog");
    require(unbreakable.totalSupply() < 10, "should not revert");
  }

  function invariantTestFail() public {
    emit log("invariantTestFailLog");
    require(breakable.totalSupply() < 10, "should revert");
  }

}
