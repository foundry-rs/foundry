pragma solidity ^0.8.10;

contract FuzzTests {
    function testFuzzedRevert(uint256 x) public {
        require(x == 5, "fuzztest-revert");
    }
}
