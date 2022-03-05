pragma solidity ^0.8.10;

contract C {
    address stateAddr = address(1337);

    function t(address _t) public {
        require(_t != stateAddr, "fuzzstate-revert");
    }
}

contract FuzzTests {
    function testFuzzedRevert(uint256 x) public {
        require(x == 5, "fuzztest-revert");
    }

    function testFuzzedStateRevert(address x) public {
        C c = new C();
        c.t(x);
    }
}
