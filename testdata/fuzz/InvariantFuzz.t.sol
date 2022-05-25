// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract UnbreakableMockToken {

    uint public totalSupply = 0;

    function shouldRevert() public {
      require(false);
    }
    function mint(uint _a) public {
        totalSupply *= _a;
    } 

    function harmless() public {
    } 
}

contract BreakableMockToken {

    uint public totalSupply = 0;

    function shouldRevert() public {
      require(false);
      totalSupply += 5;
    }
    function mint(uint _a) public {
        totalSupply += 5;
    } 
}

contract InvariantFuzzTest is DSTest {

    UnbreakableMockToken ignore;
    UnbreakableMockToken unbreakable;
    BreakableMockToken breakable;

  struct FuzzSelector {
      address addr;
      bytes4[] selectors;
  }

  function setUp() public {
      unbreakable = new UnbreakableMockToken();
      ignore = new UnbreakableMockToken();
      breakable = new BreakableMockToken();
  }

  function excludeContracts() public returns (address[] memory) {
    address[] memory addrs = new address[](2);
    addrs[0] = address(unbreakable);
    addrs[1] = address(ignore);
    return addrs;
  }

  function targetContracts() public returns (address[] memory) {
    address[] memory addrs = new address[](2);
    addrs[0] = address(breakable);
    addrs[1] = address(unbreakable);
    return addrs;
  }

  function targetSenders() public returns (address[] memory) {
    address[] memory addrs = new address[](2);
    addrs[0] = address(0x1337);
    addrs[1] = address(0x1338);
    return addrs;
  }

  function targetSelectors() public returns (FuzzSelector[] memory) {
    FuzzSelector[] memory targets = new FuzzSelector[](1);
    bytes4[] memory selectors = new bytes4[](1);
    selectors[0] = BreakableMockToken.shouldRevert.selector;
    targets[0] = FuzzSelector(address(breakable), selectors);
    return targets;
  }

  function invariantTestPass2() public {
    emit log("invariantTestPassLog");
    require(unbreakable.totalSupply() < 10, "should not revert");
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
